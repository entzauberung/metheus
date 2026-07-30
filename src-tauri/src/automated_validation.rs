use crate::project;

const MAX_CAPTURED_TEST_OUTPUT_BYTES: usize = 1_000_000;

#[derive(Debug, Clone)]
pub(crate) struct AutomatedTestEvidence {
    pub(crate) rendered: Option<String>,
    pub(crate) command: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) output_summary: String,
    pub(crate) status: project::AutomatedTestStatus,
}

impl AutomatedTestEvidence {
    pub(crate) fn not_configured(rendered: Option<String>) -> Self {
        Self {
            rendered,
            command: String::new(),
            exit_code: None,
            output_summary: String::new(),
            status: project::AutomatedTestStatus::NotConfigured,
        }
    }

    fn completed(command: &str, code: i32, summary: String, rendered: String) -> Self {
        Self {
            rendered: Some(rendered),
            command: command.to_string(),
            exit_code: Some(code),
            output_summary: summary,
            status: if code == 0 {
                project::AutomatedTestStatus::Passed
            } else {
                project::AutomatedTestStatus::Failed
            },
        }
    }

    fn unavailable(command: &str, message: String) -> Self {
        Self {
            rendered: Some(message.clone()),
            command: command.to_string(),
            exit_code: None,
            output_summary: message,
            status: project::AutomatedTestStatus::Unavailable,
        }
    }

    pub(crate) fn from_previous(previous: &project::TestResult) -> Self {
        let rendered = match previous.automated_test_status {
            project::AutomatedTestStatus::Passed | project::AutomatedTestStatus::Failed => {
                let exit_code = previous.test_exit_code.unwrap_or_else(|| {
                    if previous.automated_test_status == project::AutomatedTestStatus::Passed {
                        0
                    } else {
                        1
                    }
                });
                Some(format_test_result(
                    &previous.test_command,
                    exit_code,
                    &previous.test_output_summary,
                ))
            }
            project::AutomatedTestStatus::Unavailable => Some(previous.test_output_summary.clone()),
            project::AutomatedTestStatus::NotConfigured | project::AutomatedTestStatus::Unknown => {
                None
            }
        };
        Self {
            rendered,
            command: previous.test_command.clone(),
            exit_code: previous.test_exit_code,
            output_summary: previous.test_output_summary.clone(),
            status: previous.automated_test_status.clone(),
        }
    }
}

fn run_test_command(
    cmd: &str,
    args: &[&str],
    cwd: &str,
    timeout_secs: u64,
) -> Result<(i32, String, String), String> {
    let mut command = std::process::Command::new(cmd);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动进程 '{}': {}", cmd, error))?;
    let process_id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法捕获测试进程 stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法捕获测试进程 stderr".to_string())?;
    let stdout_reader = std::thread::spawn(move || read_bounded_tail(stdout));
    let stderr_reader = std::thread::spawn(move || read_bounded_tail(stderr));
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_process_tree(process_id);
                let stdout = stdout_reader.join().unwrap_or_default();
                let stderr = stderr_reader.join().unwrap_or_default();
                return Ok((
                    status.code().unwrap_or(-1),
                    render_captured_output(stdout),
                    render_captured_output(stderr),
                ));
            }
            Ok(None) if start.elapsed() > std::time::Duration::from_secs(timeout_secs) => {
                terminate_process_tree(process_id);
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("测试超时（超过 {} 秒），已强制终止", timeout_secs));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(500)),
            Err(error) => {
                terminate_process_tree(process_id);
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("进程异常: {}", error));
            }
        }
    }
}

fn read_bounded_tail(mut reader: impl std::io::Read) -> (Vec<u8>, bool) {
    use std::collections::VecDeque;

    let mut retained = VecDeque::with_capacity(MAX_CAPTURED_TEST_OUTPUT_BYTES);
    let mut truncated = false;
    let mut chunk = [0_u8; 8_192];
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        if retained.len() + count > MAX_CAPTURED_TEST_OUTPUT_BYTES {
            let remove = retained
                .len()
                .saturating_add(count)
                .saturating_sub(MAX_CAPTURED_TEST_OUTPUT_BYTES);
            retained.drain(..remove.min(retained.len()));
            truncated = true;
        }
        retained.extend(&chunk[..count]);
    }
    (retained.into_iter().collect(), truncated)
}

fn render_captured_output((bytes, truncated): (Vec<u8>, bool)) -> String {
    let output = String::from_utf8_lossy(&bytes);
    if truncated {
        format!(
            "[测试输出已截断，仅保留最后 {} 字节]\n{}",
            MAX_CAPTURED_TEST_OUTPUT_BYTES, output
        )
    } else {
        output.to_string()
    }
}

fn terminate_process_tree(process_id: u32) {
    #[cfg(unix)]
    {
        let group = format!("-{process_id}");
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &group])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

pub(crate) fn summarize_test_output(exit_code: i32, stdout: &str, stderr: &str) -> String {
    let combined = format!("{}{}", stdout, stderr);
    if exit_code == 0 {
        if combined.chars().count() > 500 {
            let suffix = combined
                .chars()
                .rev()
                .take(500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!(
                "…(省略前面 {} 字符)…\n\n{}",
                combined.chars().count().saturating_sub(500),
                suffix
            )
        } else {
            combined
        }
    } else {
        let keywords = ["FAIL", "Error", "失败", "error", "panic", "Exception"];
        let best_pos = keywords
            .iter()
            .filter_map(|keyword| combined.rfind(keyword))
            .max();
        if let Some(byte_pos) = best_pos {
            let char_index = combined[..byte_pos].chars().count();
            let total = combined.chars().count();
            let start = char_index.saturating_sub(500);
            let end = (char_index + 500).min(total);
            let snippet = combined
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>();
            format!("退出码: {}\n\n{}", exit_code, snippet)
        } else {
            let tail = combined
                .chars()
                .rev()
                .take(3_000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!("退出码: {}\n\n{}", exit_code, tail)
        }
    }
}

fn format_test_result(command: &str, exit_code: i32, summary: &str) -> String {
    let status = if exit_code == 0 { "通过" } else { "失败" };
    format!(
        "测试命令: {}\n状态: {} (exit code: {})\n\n输出:\n{}",
        command, status, exit_code, summary
    )
}

fn is_test_not_configured(stderr: &str, stdout: &str) -> bool {
    let combined = format!("{}{}", stderr, stdout).to_ascii_lowercase();
    if combined.contains("missing script: test")
        || combined.contains("no tests found")
        || combined.contains("no test specified")
        || combined.contains("no test files found")
        || combined.contains("no tests were found")
        || combined.contains("no tests ran")
        || combined.contains("collected 0 items")
        || combined.contains("ran 0 tests")
        || combined.contains("no tests to run")
    {
        return true;
    }
    let cargo_counts = combined
        .lines()
        .filter_map(|line| line.trim().strip_prefix("running "))
        .filter_map(|suffix| suffix.split_whitespace().next())
        .filter_map(|count| count.parse::<u32>().ok())
        .collect::<Vec<_>>();
    if !cargo_counts.is_empty() && cargo_counts.iter().all(|count| *count == 0) {
        return true;
    }
    let go_packages = combined
        .lines()
        .filter(|line| line.starts_with("?\t") || line.starts_with("ok\t"))
        .collect::<Vec<_>>();
    if !go_packages.is_empty()
        && go_packages
            .iter()
            .all(|line| line.contains("[no test files]"))
    {
        return true;
    }
    let gradle_test_tasks = combined
        .lines()
        .filter(|line| line.contains("> task") && line.contains("test"))
        .collect::<Vec<_>>();
    !gradle_test_tasks.is_empty()
        && gradle_test_tasks
            .iter()
            .all(|line| line.contains("no-source"))
}

fn completed(
    command: &str,
    result: Result<(i32, String, String), String>,
) -> AutomatedTestEvidence {
    match result {
        Ok((code, stdout, stderr)) => {
            let summary = summarize_test_output(code, &stdout, &stderr);
            if is_test_not_configured(&stderr, &stdout) {
                return AutomatedTestEvidence {
                    rendered: Some(format!(
                        "测试命令: {command}\n状态: 未配置测试\n\n{summary}"
                    )),
                    command: command.to_string(),
                    exit_code: Some(code),
                    output_summary: summary,
                    status: project::AutomatedTestStatus::NotConfigured,
                };
            }
            let rendered = format_test_result(command, code, &summary);
            AutomatedTestEvidence::completed(command, code, summary, rendered)
        }
        Err(error) => AutomatedTestEvidence::unavailable(
            command,
            format!("{} 执行失败（测试环境不可用）：{}", command, error),
        ),
    }
}

pub(crate) fn run_project_tests(project_path: &str) -> AutomatedTestEvidence {
    let root = std::path::Path::new(project_path);
    let metheus_test = root.join(".metheus-test");
    if metheus_test.exists() {
        return match std::fs::read_to_string(&metheus_test) {
            Ok(contents) => {
                let command = contents.trim();
                if command.is_empty() || command.starts_with('#') {
                    AutomatedTestEvidence::not_configured(None)
                } else {
                    let parts = command.split_whitespace().collect::<Vec<_>>();
                    completed(
                        command,
                        run_test_command(parts[0], &parts[1..], project_path, 300),
                    )
                }
            }
            Err(error) => AutomatedTestEvidence::unavailable(
                ".metheus-test",
                format!("读取 .metheus-test 失败：{}", error),
            ),
        };
    }
    if root.join("package.json").exists() {
        let manager = if root.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if root.join("yarn.lock").exists() {
            "yarn"
        } else {
            "npm"
        };
        let command = format!("{} test", manager);
        return completed(
            &command,
            run_test_command(manager, &["test"], project_path, 300),
        );
    }
    if root.join("Cargo.toml").exists() {
        return completed(
            "cargo test",
            run_test_command("cargo", &["test"], project_path, 600),
        );
    }
    if root.join("go.mod").exists() {
        return completed(
            "go test ./...",
            run_test_command("go", &["test", "./..."], project_path, 300),
        );
    }
    if root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("setup.cfg").exists()
    {
        let pytest = std::process::Command::new("python")
            .args(["-m", "pytest", "--version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        let args = if pytest {
            vec!["-m", "pytest"]
        } else {
            vec!["-m", "unittest", "discover"]
        };
        let command = format!("python {}", args.join(" "));
        return completed(
            &command,
            run_test_command("python", &args, project_path, 300),
        );
    }
    if root.join("CMakeLists.txt").exists() {
        return completed("ctest", run_test_command("ctest", &[], project_path, 300));
    }
    if root.join("pom.xml").exists() {
        return completed(
            "mvn test",
            run_test_command("mvn", &["test"], project_path, 600),
        );
    }
    if root.join("build.gradle").exists() || root.join("build.gradle.kts").exists() {
        let executable = if cfg!(windows) {
            "gradlew.bat"
        } else {
            "./gradlew"
        };
        return completed(
            "gradle test",
            run_test_command(executable, &["test"], project_path, 600),
        );
    }
    AutomatedTestEvidence::not_configured(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_project_has_no_configured_tests() -> Result<(), String> {
        let path = std::env::temp_dir().join(format!("metheus-no-tests-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let evidence = run_project_tests(&path.to_string_lossy());
        assert_eq!(evidence.status, project::AutomatedTestStatus::NotConfigured);
        assert!(evidence.command.is_empty());
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn previous_test_facts_are_reused_without_execution() {
        let previous = project::TestResult {
            test_command: "cargo test --lib".into(),
            test_exit_code: Some(0),
            test_output_summary: "12 passed".into(),
            automated_test_status: project::AutomatedTestStatus::Passed,
            ..Default::default()
        };
        let evidence = AutomatedTestEvidence::from_previous(&previous);
        assert_eq!(evidence.status, project::AutomatedTestStatus::Passed);
        assert!(evidence
            .rendered
            .is_some_and(|text| text.contains("12 passed")));
    }

    #[test]
    fn zero_test_outputs_are_not_reported_as_passed() {
        for (command, stdout) in [
            ("ctest", "Test project /tmp/build\nNo tests were found!!!\n"),
            (
                "cargo test",
                "running 0 tests\n\ntest result: ok. 0 passed; 0 failed\n",
            ),
            ("go test ./...", "?\texample/project\t[no test files]\n"),
            ("python -m pytest", "collected 0 items\n\nno tests ran\n"),
            ("mvn test", "No tests to run.\nBUILD SUCCESS\n"),
            ("gradle test", "> Task :test NO-SOURCE\nBUILD SUCCESSFUL\n"),
        ] {
            let evidence = completed(command, Ok((0, stdout.to_string(), String::new())));
            assert_eq!(
                evidence.status,
                project::AutomatedTestStatus::NotConfigured,
                "{command}"
            );
        }
    }

    #[test]
    fn positive_cargo_test_count_is_still_passed() {
        let evidence = completed(
            "cargo test",
            Ok((
                0,
                "running 0 tests\nrunning 3 tests\ntest result: ok. 3 passed\n".into(),
                String::new(),
            )),
        );
        assert_eq!(evidence.status, project::AutomatedTestStatus::Passed);
    }

    #[test]
    fn captured_output_is_bounded_and_keeps_the_tail() {
        let mut input = vec![b'a'; MAX_CAPTURED_TEST_OUTPUT_BYTES + 32];
        input.extend_from_slice(b"tail-marker");
        let captured = read_bounded_tail(std::io::Cursor::new(input));
        assert!(captured.1);
        assert_eq!(captured.0.len(), MAX_CAPTURED_TEST_OUTPUT_BYTES);
        assert!(captured.0.ends_with(b"tail-marker"));
    }
}
