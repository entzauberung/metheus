use crate::project;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};

pub(crate) type FileSnapshot = BTreeMap<String, u64>;

fn display_path(path: &str, project_path: &str) -> String {
    std::path::Path::new(path)
        .strip_prefix(project_path)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Compare content fingerprints so additions, modifications, and deletions are all visible.
pub(crate) fn detect_changes(
    before: &FileSnapshot,
    after: &FileSnapshot,
    project_path: &str,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .map(|path| display_path(path, project_path))
        .collect()
}

#[cfg(test)]
mod change_detection_tests {
    use super::{detect_changes, FileSnapshot};

    #[test]
    fn detects_added_modified_and_deleted_files() {
        let before = FileSnapshot::from([
            ("/project/deleted.rs".to_string(), 1),
            ("/project/modified.rs".to_string(), 2),
            ("/project/unchanged.rs".to_string(), 3),
        ]);
        let after = FileSnapshot::from([
            ("/project/added.rs".to_string(), 4),
            ("/project/modified.rs".to_string(), 5),
            ("/project/unchanged.rs".to_string(), 3),
        ]);

        assert_eq!(
            detect_changes(&before, &after, "/project"),
            vec!["added.rs", "deleted.rs", "modified.rs"]
        );
    }
}
/// 调用方（如 check_subtask）
///    ↓
/// run_test_command("cargo", &["test"], "/project", 120)  → 得到 (code, stdout, stderr)
///    ↓
/// summarize_test_output(code, &stdout, &stderr)           → 得到精简摘要
///    ↓
/// format_test_result("cargo test", code, &summary)        → 得到最终字符串
///    ↓
/// 返回给 AI 或前端
/// 模拟一个"测试工程师"角色：
/// 自动检查当前项目里所有改动的代码，判断是否达到了子任务的目标，并返回测试结果（通过/问题/建议）
/// 扫描项目目录，递归返回所有文件路径列表（跳过 .git / node_modules / target）
pub(crate) fn get_tracked_files(project_path: &str) -> Vec<String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(project_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path().to_string_lossy().to_string();
        // 跳过 .git / node_modules / target 目录及其内部
        if path.contains("/.git/")
            || path.contains("/node_modules/")
            || path.contains("/target/")
            || path.ends_with("/.git")
            || path.ends_with("/node_modules")
            || path.ends_with("/target")
        {
            continue;
        }
        // 只记录文件
        if entry.file_type().is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
}

pub(crate) fn get_file_snapshot(project_path: &str) -> FileSnapshot {
    get_tracked_files(project_path)
        .into_iter()
        .map(|path| {
            let mut hasher = DefaultHasher::new();
            match std::fs::read(&path) {
                Ok(content) => content.hash(&mut hasher),
                Err(error) => error.kind().hash(&mut hasher),
            }
            (path, hasher.finish())
        })
        .collect()
}
/// 执行测试命令，带超时控制（spawn + try_wait 轮询）
/// 返回: (exit_code, stdout, stderr)
/// 测试辅助函数
pub(crate) fn run_test_command(
    cmd: &str,
    args: &[&str],
    cwd: &str,
    timeout_secs: u64,
) -> Result<(i32, String, String), String> {
    use std::io::Read;
    // 创建子进程，以便捕获输出
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动进程 '{}': {}", cmd, e))?;

    // Read both pipes concurrently; otherwise a full pipe can block the child before it exits.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法捕获测试进程 stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法捕获测试进程 stderr".to_string())?;
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut reader = stdout;
        let _ = reader.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut reader = stderr;
        let _ = reader.read_to_end(&mut bytes);
        bytes
    });
    // 记录开始时间
    let start = std::time::Instant::now();
    // 进入循环
    loop {
        // 检查进程是否结束（非阻塞）
        match child.try_wait() {
            // 如果已结束（Ok(Some(status))）：读取 stdout/stderr 剩余内容，返回 (exit_code, stdout, stderr)
            Ok(Some(status)) => {
                let stdout = stdout_reader
                    .join()
                    .map_err(|_| "读取测试进程 stdout 的线程异常".to_string())?;
                let stderr = stderr_reader
                    .join()
                    .map_err(|_| "读取测试进程 stderr 的线程异常".to_string())?;
                return Ok((
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&stdout).into_owned(),
                    String::from_utf8_lossy(&stderr).into_owned(),
                ));
            }
            // 如果还在运行（Ok(None)）：检查是否超时，若超时则 child.kill() 并返回错误；否则休眠 500 毫秒后继续轮询
            Ok(None) => {
                if start.elapsed() > std::time::Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(format!("测试超时（超过 {} 秒），已强制终止", timeout_secs));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // 出错（Err(e)）：终止进程并返回错误
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("进程异常: {}", e));
            }
        }
    }
}

/// 从测试输出中提取关键信息
/// 通过 → 截取最后 500 字符
/// 失败 → 截取最后 3000 字符 + 提取含失败关键词的行
pub(crate) fn summarize_test_output(exit_code: i32, stdout: &str, stderr: &str) -> String {
    let combined = format!("{}{}", stdout, stderr);
    // 如果测试通过（exit_code == 0）：只保留最后 500 个字符
    if exit_code == 0 {
        // 通过：保留最后 500 字符
        if combined.len() > 500 {
            let suffix: String = combined
                .chars()
                .rev()
                .take(500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!(
                "…(省略前面 {} 字符)…\n\n{}",
                combined.chars().count().saturating_sub(500),
                suffix
            )
        } else {
            combined
        }
    // 如果测试失败（exit_code != 0）
    } else {
        // 失败：搜索关键词，截取失败点附近的上下文
        let keywords = ["FAIL", "Error", "失败", "error", "panic", "Exception"];

        // 从末尾向前搜索关键词，取最靠近末尾的匹配位置
        let mut best_pos: Option<usize> = None;
        for kw in &keywords {
            if let Some(pos) = combined.rfind(kw) {
                match best_pos {
                    None => best_pos = Some(pos),
                    Some(current) if pos > current => best_pos = Some(pos),
                    _ => {}
                }
            }
        }

        match best_pos {
            Some(kw_byte_pos) => {
                // 将字节偏移转换为字符索引
                let kw_char_idx = combined[..kw_byte_pos].chars().count();
                let total_chars = combined.chars().count();
                let start_char = kw_char_idx.saturating_sub(500);
                let end_char = (kw_char_idx + 500).min(total_chars);
                let snippet: String = combined
                    .chars()
                    .skip(start_char)
                    .take(end_char - start_char)
                    .collect();
                format!("退出码: {}\n\n{}", exit_code, snippet)
            }
            None => {
                // 回退：未找到关键词，截取最后 3000 字符
                let tail: String = if combined.len() > 3000 {
                    combined
                        .chars()
                        .rev()
                        .take(3000)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect()
                } else {
                    combined
                };
                format!("退出码: {}\n\n{}", exit_code, tail)
            }
        }
    }
}

/// 格式化测试结果
pub(crate) fn format_test_result(_label: &str, command: &str, exit_code: i32, summary: &str) -> String {
    let status = if exit_code == 0 {
        "✅ 通过"
    } else {
        "❌ 失败"
    };
    format!(
        "测试命令: {}\n状态: {} (exit code: {})\n\n输出:\n{}",
        command, status, exit_code, summary
    )
}

/// 检查 stderr 是否表明测试未配置（而非测试失败）
pub(crate) fn is_test_not_configured(stderr: &str, stdout: &str) -> bool {
    let combined = format!("{}{}", stderr, stdout);
    combined.contains("missing script: test")
        || combined.contains("No tests found")
        || combined.contains("no test specified")
        || combined.contains("No test files found")
}

/// 测试
#[tauri::command]
pub(crate) async fn check_subtask(
    project_path: &str,
    subtask_goal: &str,
    _subtask_id: &str,
    _milestone_id: &str,
    _mid_stage_id: &str,
) -> Result<project::TestResult, String> {
    // 1.尝试 git diff --name-only 获取改动文件
    let files: Vec<String> = {
        let git_result = std::process::Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(&project_path)
            .output();

        match git_result {
            Ok(output) if output.status.success() => {
                // git 命令成功，解析 stdout
                let changed = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>();
                if !changed.is_empty() {
                    changed
                } else {
                    // git diff 成功但无变更（工作区干净），降级走文件系统
                    vec![]
                }
            }
            _ => {
                // git 命令失败（非仓库/未安装git/其他错误），降级走文件系统
                vec![]
            }
        }
    };

    // 2.如果 git diff 没能拿到文件列表，降级：扫描项目目录中的源文件
    let files = if files.is_empty() {
        walkdir::WalkDir::new(&project_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let path = e.path().strip_prefix(&project_path).ok()?;
                let ext = path.extension()?.to_str()?;
                // 只收集常见源代码文件
                match ext {
                    "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp"
                    | "h" | "hpp" | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "vue"
                    | "svelte" | "html" | "css" | "scss" | "json" | "yaml" | "yml" | "toml"
                    | "md" | "txt" => Some(path.to_string_lossy().to_string()),
                    _ => None,
                }
            })
            .collect::<Vec<String>>()
    } else {
        files
    };

    // 3.遍历每个文件，读取内容（限制总大小防止爆 token）
    let mut file_contents = String::new();
    const MAX_CONTENT_BYTES: usize = 30_000; // 约 7500 个中文字符
    for file in &files {
        if file_contents.len() >= MAX_CONTENT_BYTES {
            break;
        }
        let content = std::fs::read_to_string(std::path::Path::new(&project_path).join(file))
            .unwrap_or_default();
        let truncated = if content.len() > 4000 {
            let prefix: String = content.chars().take(1000).collect();
            format!(
                "{}...(省略后续 {} 字符)",
                prefix,
                content.chars().count().saturating_sub(1000)
            )
        } else {
            content
        };
        file_contents.push_str(&format!("\n=== {} ===\n{}\n", file, truncated));
    }
    // ===== 真测试：检测项目类型，执行对应的测试命令 =====
    let test_output: Option<String> = {
        let project_root = std::path::Path::new(project_path);

        // 优先检测自定义测试命令文件 .metheus-test
        let metheus_test_file = project_root.join(".metheus-test");
        if metheus_test_file.exists() {
            match std::fs::read_to_string(&metheus_test_file) {
                Ok(contents) => {
                    let cmd_line = contents.trim().to_string();
                    if cmd_line.is_empty() || cmd_line.starts_with('#') {
                        eprintln!("[check_subtask] .metheus-test 为空或注释，跳过");
                        None
                    } else {
                        let parts: Vec<&str> = cmd_line.split_whitespace().collect();
                        let cmd = parts[0];
                        let cmd_args = &parts[1..];
                        eprintln!("[check_subtask] 使用自定义测试命令: {}", cmd_line);
                        match run_test_command(cmd, cmd_args, project_path, 300) {
                            Ok((code, stdout, stderr)) => {
                                let summary = summarize_test_output(code, &stdout, &stderr);
                                Some(format_test_result("自定义测试", &cmd_line, code, &summary))
                            }
                            Err(e) => Some(format!("自定义测试执行失败：{}", e)),
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[check_subtask] 读取 .metheus-test 失败: {}", e);
                    None
                }
            }
        } else if project_root.join("package.json").exists() {
            // JS/TS 项目：自动识别包管理器
            let pm = if project_root.join("pnpm-lock.yaml").exists() {
                "pnpm"
            } else if project_root.join("yarn.lock").exists() {
                "yarn"
            } else {
                "npm"
            };
            let label = format!("{} test", pm);
            match run_test_command(pm, &["test"], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    if code != 0 && is_test_not_configured(&stderr, &stdout) {
                        let stderr_preview: String = stderr.chars().take(200).collect();
                        Some(format!(
                            "测试命令: {}\n状态: ⚠️ 未配置测试用例（{} 返回：{}）\n\n该项目未配置测试用例，请仅基于代码审查判定，不要将此视为测试失败。",
                            label, pm, stderr_preview
                        ))
                    } else {
                        let summary = summarize_test_output(code, &stdout, &stderr);
                        Some(format_test_result(&label, &label, code, &summary))
                    }
                }
                Err(e) => Some(format!("{} test 执行失败（测试环境未配置）：{}", pm, e)),
            }
        } else if project_root.join("Cargo.toml").exists() {
            // Rust 项目
            match run_test_command("cargo", &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(
                        "cargo test",
                        "cargo test",
                        code,
                        &summary,
                    ))
                }
                Err(e) => Some(format!("cargo test 执行失败：{}", e)),
            }
        } else if project_root.join("go.mod").exists() {
            // Go 项目
            match run_test_command("go", &["test", "./..."], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(
                        "go test",
                        "go test ./...",
                        code,
                        &summary,
                    ))
                }
                Err(e) => Some(format!("go test 执行失败：{}", e)),
            }
        } else if project_root.join("pyproject.toml").exists()
            || project_root.join("setup.py").exists()
            || project_root.join("setup.cfg").exists()
        {
            // Python 项目：先检测 pytest 是否可用
            let (cmd, args): (&str, Vec<&str>) = if std::process::Command::new("python")
                .args(["-m", "pytest", "--version"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                ("python", vec!["-m", "pytest"])
            } else {
                ("python", vec!["-m", "unittest", "discover"])
            };
            let label = if args.contains(&"pytest") {
                "pytest"
            } else {
                "unittest"
            };
            let full_cmd = format!("{} {}", cmd, args.join(" "));
            let args_slice: Vec<&str> = args.iter().map(|s| *s).collect();
            match run_test_command(cmd, &args_slice, project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(label, &full_cmd, code, &summary))
                }
                Err(e) => Some(format!("{} 执行失败：{}", label, e)),
            }
        } else if project_root.join("CMakeLists.txt").exists() {
            // C++ 项目
            match run_test_command("ctest", &[], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result("ctest", "ctest", code, &summary))
                }
                Err(e) => Some(format!("ctest 执行失败：{}", e)),
            }
        } else if project_root.join("pom.xml").exists() {
            // Java Maven
            match run_test_command("mvn", &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result("mvn test", "mvn test", code, &summary))
                }
                Err(e) => Some(format!("mvn test 执行失败：{}", e)),
            }
        } else if project_root.join("build.gradle").exists()
            || project_root.join("build.gradle.kts").exists()
        {
            // Java Gradle
            let gradle_cmd = if cfg!(windows) {
                "gradlew.bat"
            } else {
                "./gradlew"
            };
            match run_test_command(gradle_cmd, &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    Some(format_test_result(
                        "gradle test",
                        "gradle test",
                        code,
                        &summary,
                    ))
                }
                Err(e) => Some(format!("gradle test 执行失败：{}", e)),
            }
        } else {
            eprintln!("[check_subtask] 未检测到已知测试框架，跳过真测试");
            None
        }
    };
    // Mock 版本 -> 3.4.1c改动
    // 构建测试工程师prompt 的 user_message
    // eprintln!("[check_subtask] 测试结果注入完成, test_output 长度: {}",
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 构造子任务目标描述（注入给测试工程师 AI）
    let goal_section = if subtask_goal.is_empty() {
        "## 子任务目标\n（未提供子任务目标描述，请仅根据代码变更做通用质量检查）\n\n".to_string()
    } else {
        let truncated: String = subtask_goal.chars().take(2000).collect();
        let suffix = if subtask_goal.chars().count() > 2000 {
            "…（已截断）"
        } else {
            ""
        };
        format!(
            "## 子任务目标\n{}\n{}\n请根据以上目标，检查下列代码变更是否完整、正确地实现了该目标。\n\n",
            truncated, suffix
        )
    };
    let user_message = if let Some(ref test_result) = test_output {
        format!(
            "{}请检查以下代码改动。\n\n## 自动化测试结果\n项目自动化测试已执行，结果如下：\n\n{}\n\n---\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            goal_section,
            test_result,
            files.len(),
            files.join("\n"),
            file_contents
        )
    } else {
        format!(
            "{}请检查以下代码改动：\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            goal_section,
            files.len(),
            files.join("\n"),
            file_contents
        )
    };
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 调用 AI（强制 JSON 模式）
    let mut diagnosis_warnings: Vec<String> = Vec::new();
    let raw_reply = crate::api::call_deepseek_api_json(crate::prompts::TEST_PROMPT, &user_message)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[check_subtask] AI API 调用失败：{}，返回兜底 JSON", e);
            diagnosis_warnings.push(format!("AI API 调用失败：{}", e));
            r#"{"passed": false, "issues": ["AI API 调用失败"], "suggestion": "", "warnings": []}"#.to_string()
        });
    // 解析 JSON 响应（带兜底）
    let test_result: project::TestResult =
        match crate::json_utils::parse_json_with_retry::<project::TestResult>(&raw_reply).await {
            Ok(mut result) => {
                result.warnings.extend(diagnosis_warnings);
                result
            }
            Err(e) => {
                eprintln!(
                    "[check_subtask] TestResult JSON 解析失败：{}，使用默认失败结果",
                    e
                );
                let preview: String = raw_reply.chars().take(200).collect();
                diagnosis_warnings.push(format!(
                    "TestResult JSON 解析失败：{}。原始内容（前200字符）：{}",
                    e, preview
                ));
                project::TestResult {
                    passed: false,
                    issues: vec![format!(
                        "AI 返回格式异常，解析失败：{}。原始内容（前200字符）：{}",
                        e, preview
                    )],
                    suggestion: "AI 返回格式异常，请人工审查".to_string(),
                    warnings: diagnosis_warnings,
                }
            }
        };
    Ok(test_result)
}
