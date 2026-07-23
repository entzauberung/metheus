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
    use super::{
        build_review_evidence, detect_changes, git_changed_files, truncate_head_tail, FileSnapshot,
    };
    use crate::project::ReviewEvidenceStatus;
    use std::process::Command;

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

    #[test]
    fn git_changed_files_includes_tracked_and_untracked_evidence() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-test-evidence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| format!("创建测试目录失败：{}", error))?;
        let git = |args: &[&str]| -> Result<(), String> {
            let output = Command::new("git")
                .args(args)
                .current_dir(&path)
                .output()
                .map_err(|error| format!("运行 git 失败：{}", error))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        };
        git(&["init", "--quiet"])?;
        git(&["config", "user.name", "Metheus Test"])?;
        git(&["config", "user.email", "metheus-test@example.invalid"])?;
        std::fs::write(path.join("tracked.rs"), "fn before() {}\n")
            .map_err(|error| error.to_string())?;
        git(&["add", "tracked.rs"])?;
        git(&["commit", "--quiet", "-m", "baseline"])?;
        std::fs::write(path.join("tracked.rs"), "fn after() {}\n")
            .map_err(|error| error.to_string())?;
        std::fs::write(path.join("new.rs"), "fn new_file() {}\n")
            .map_err(|error| error.to_string())?;

        let project_path = path.to_string_lossy().to_string();
        assert_eq!(
            git_changed_files(&project_path),
            vec!["new.rs".to_string(), "tracked.rs".to_string()]
        );
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn long_html_evidence_keeps_script_changes_at_file_tail() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-review-html-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let git = |args: &[&str]| -> Result<(), String> {
            let output = Command::new("git")
                .args(args)
                .current_dir(&path)
                .output()
                .map_err(|error| error.to_string())?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        };
        git(&["init", "--quiet"])?;
        git(&["config", "user.name", "Metheus Test"])?;
        git(&["config", "user.email", "metheus-test@example.invalid"])?;
        let baseline = format!(
            "<html>\n<body>\n{}</body>\n</html>\n",
            "<div>line</div>\n".repeat(300)
        );
        std::fs::write(path.join("index.html"), &baseline).map_err(|error| error.to_string())?;
        git(&["add", "index.html"])?;
        git(&["commit", "--quiet", "-m", "baseline"])?;
        let updated = baseline.replace(
            "</body>",
            "<script>\nfunction toggleTheme() { document.body.classList.toggle('dark'); }\n</script>\n</body>",
        );
        std::fs::write(path.join("index.html"), updated).map_err(|error| error.to_string())?;

        let evidence = build_review_evidence(
            &path.to_string_lossy(),
            &["index.html".to_string()],
            &["toggleTheme()".to_string()],
        );
        assert!(evidence.rendered.contains("function toggleTheme"));
        assert!(evidence.rendered.contains("Git diff"));
        assert_eq!(evidence.status, ReviewEvidenceStatus::Partial);
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn long_file_evidence_includes_identifier_context_from_middle() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-review-context-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let content = format!(
            "{}\nfunction targetHandler(event) {{ event.preventDefault(); }}\n{}",
            "const filler = 1;\n".repeat(500),
            "const tail = 1;\n".repeat(500)
        );
        std::fs::write(path.join("index.html"), content).map_err(|error| error.to_string())?;
        let evidence = build_review_evidence(
            &path.to_string_lossy(),
            &["index.html".to_string()],
            &["event.preventDefault".to_string()],
        );
        assert!(evidence.rendered.contains("验收标识符命中上下文"));
        assert!(evidence.rendered.contains("targetHandler"));
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn evidence_truncation_respects_unicode_boundaries_and_marks_omission() {
        let input = "审".repeat(1_000);
        let (rendered, truncated) = truncate_head_tail(&input, 160);
        assert!(truncated);
        assert!(rendered.contains("证据截断"));
        assert!(rendered.chars().count() <= 160);
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

fn git_changed_files(project_path: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for args in [
        vec!["diff", "--name-only", "-z", "HEAD"],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        if let Ok(output) = std::process::Command::new("git")
            .args(args)
            .current_dir(project_path)
            .output()
        {
            if output.status.success() {
                files.extend(
                    output
                        .stdout
                        .split(|byte| *byte == 0)
                        .filter(|path| !path.is_empty())
                        .map(|path| String::from_utf8_lossy(path).to_string()),
                );
            }
        }
    }
    files.into_iter().collect()
}

const MAX_REVIEW_EVIDENCE_CHARS: usize = 30_000;
const MAX_FILE_EVIDENCE_CHARS: usize = 8_000;
const FULL_FILE_PREVIEW_CHARS: usize = 4_000;

#[derive(Debug)]
struct ReviewEvidence {
    rendered: String,
    status: project::ReviewEvidenceStatus,
    summary: String,
}

fn merge_evidence_status(
    current: &mut project::ReviewEvidenceStatus,
    next: project::ReviewEvidenceStatus,
) {
    use project::ReviewEvidenceStatus::{Complete, Partial, Unavailable};
    if matches!(next, Unavailable) || matches!((&*current, next), (Complete, Partial)) {
        *current = next;
    }
}

fn truncate_head_tail(text: &str, limit: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= limit {
        return (text.to_string(), false);
    }
    if limit < 80 {
        return (text.chars().take(limit).collect(), true);
    }

    let marker_reserve = 60.min(limit / 3);
    let content_budget = limit.saturating_sub(marker_reserve);
    let head_budget = content_budget / 2;
    let tail_budget = content_budget.saturating_sub(head_budget);
    let head: String = text.chars().take(head_budget).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let omitted = total.saturating_sub(head.chars().count() + tail.chars().count());
    (
        format!("{head}\n...[证据截断：省略 {omitted} 个字符]...\n{tail}"),
        true,
    )
}

fn number_lines(text: &str, starting_line: usize) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| format!("{:>6} | {}", starting_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_file_preview(content: &str) -> (String, bool) {
    let total = content.chars().count();
    if total <= FULL_FILE_PREVIEW_CHARS {
        return (number_lines(content, 1), false);
    }

    let head_budget = 1_000;
    let tail_budget = 3_000;
    let head: String = content.chars().take(head_budget).collect();
    let tail: String = content
        .chars()
        .rev()
        .take(tail_budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let tail_start = content.lines().count().saturating_sub(tail.lines().count()) + 1;
    let omitted = total.saturating_sub(head.chars().count() + tail.chars().count());
    (
        format!(
            "{}\n...[文件内容省略 {omitted} 个字符；省略区域不能作为代码不存在的依据]...\n{}",
            number_lines(&head, 1),
            number_lines(&tail, tail_start),
        ),
        true,
    )
}

fn git_diff_for_file(project_path: &str, file: &str) -> Result<String, String> {
    let literal_pathspec = format!(":(literal){file}");
    let output = std::process::Command::new("git")
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=20",
            "HEAD",
            "--",
            &literal_pathspec,
        ])
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("运行 git diff 失败：{error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!(
            "git diff 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn identifier_context(content: &str, identifiers: &[String]) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let mut rendered = Vec::new();
    let mut seen = BTreeSet::new();
    for identifier in identifiers {
        let needle = identifier
            .split('.')
            .next_back()
            .unwrap_or(identifier)
            .trim_end_matches("()");
        if needle.len() < 3 {
            continue;
        }
        for (index, _) in lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(needle))
            .take(3)
        {
            let start = index.saturating_sub(2);
            let end = (index + 3).min(lines.len());
            let key = format!("{start}:{end}");
            if seen.insert(key) {
                rendered.push(format!(
                    "[标识符 {identifier}，第 {}-{} 行]\n{}",
                    start + 1,
                    end,
                    number_lines(&lines[start..end].join("\n"), start + 1),
                ));
            }
        }
    }
    rendered.join("\n")
}

fn build_review_evidence(
    project_path: &str,
    files: &[String],
    identifiers: &[String],
) -> ReviewEvidence {
    if files.is_empty() {
        return ReviewEvidence {
            rendered: "（没有可供审查的改动文件）".to_string(),
            status: project::ReviewEvidenceStatus::Unavailable,
            summary: "没有可供审查的改动文件".to_string(),
        };
    }

    let mut rendered = String::new();
    let mut status = project::ReviewEvidenceStatus::Complete;
    let mut notes = Vec::new();

    for (index, file) in files.iter().enumerate() {
        let remaining_files = files.len().saturating_sub(index).max(1);
        let remaining_budget = MAX_REVIEW_EVIDENCE_CHARS.saturating_sub(rendered.chars().count());
        if remaining_budget < 200 {
            merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
            notes.push(format!("另有 {} 个文件因总预算未展开", remaining_files));
            break;
        }
        let file_budget = (remaining_budget / remaining_files)
            .min(MAX_FILE_EVIDENCE_CHARS)
            .max(200);
        let mut section = format!("\n=== {file} ===\n");
        let mut has_diff = false;

        match git_diff_for_file(project_path, file) {
            Ok(diff) if !diff.trim().is_empty() => {
                has_diff = true;
                section.push_str("[Git diff，变更事实]\n");
                section.push_str(&diff);
            }
            Ok(_) => section.push_str("[Git diff 为空，以下当前文件内容为主要证据]\n"),
            Err(error) => {
                merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Unavailable);
                notes.push(format!("{file}: {error}"));
                section.push_str(&format!("[Git diff 不可用：{error}]\n"));
            }
        }

        let full_path = std::path::Path::new(project_path).join(file);
        if full_path.exists() {
            match std::fs::read_to_string(&full_path) {
                Ok(content) => {
                    let (preview, partial) = render_file_preview(&content);
                    section.push_str("\n[当前文件内容，带行号]\n");
                    section.push_str(&preview);
                    let context = identifier_context(&content, identifiers);
                    if !context.is_empty() {
                        section.push_str("\n\n[验收标识符命中上下文]\n");
                        section.push_str(&context);
                    }
                    if partial {
                        merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
                        notes.push(format!("{file}: 当前文件仅提供头尾上下文"));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
                    notes.push(format!("{file}: 二进制或非 UTF-8 文件"));
                    section.push_str("\n[当前文件为二进制或非 UTF-8，未展开文本内容]\n");
                }
                Err(error) => {
                    merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Unavailable);
                    notes.push(format!("{file}: 读取失败：{error}"));
                    section.push_str(&format!("\n[当前文件读取失败：{error}]\n"));
                }
            }
        } else {
            if !has_diff {
                merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Unavailable);
                notes.push(format!("{file}: 文件不存在且没有 Git diff"));
            }
            section.push_str("\n[当前文件不存在，可能为本次删除]\n");
        }

        let (section, truncated) = truncate_head_tail(&section, file_budget);
        if truncated {
            merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
            notes.push(format!("{file}: 单文件证据超过 {file_budget} 字符"));
        }
        rendered.push_str(&section);
        rendered.push('\n');
    }

    let status_label = match status {
        project::ReviewEvidenceStatus::Complete => "完整",
        project::ReviewEvidenceStatus::Partial => "部分",
        project::ReviewEvidenceStatus::Unavailable => "不可用",
    };
    let summary = if notes.is_empty() {
        format!("证据{status_label}，覆盖 {} 个文件", files.len())
    } else {
        format!("证据{status_label}：{}", notes.join("；"))
    };
    ReviewEvidence {
        rendered,
        status,
        summary,
    }
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
pub(crate) fn format_test_result(
    _label: &str,
    command: &str,
    exit_code: i32,
    summary: &str,
) -> String {
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

#[derive(Debug, Clone)]
struct AutomatedTestEvidence {
    rendered: Option<String>,
    command: String,
    exit_code: Option<i32>,
    output_summary: String,
    status: project::AutomatedTestStatus,
}

impl AutomatedTestEvidence {
    fn not_configured(rendered: Option<String>) -> Self {
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
}

/// 测试
#[tauri::command]
pub(crate) async fn check_subtask(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
) -> Result<project::TestResult, String> {
    check_subtask_with_context(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        None,
        None,
        None,
    )
    .await
}

pub(crate) async fn check_subtask_with_context(
    project_path: &str,
    subtask_goal: &str,
    _subtask_id: &str,
    _milestone_id: &str,
    _mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
) -> Result<project::TestResult, String> {
    // 1.尝试 git diff --name-only 获取改动文件
    let files = git_changed_files(project_path);

    // 2.如果 git diff 没能拿到文件列表，降级：扫描项目目录中的源文件
    let files = if files.is_empty() {
        if let Some(paths) = authorized_paths.as_ref().filter(|paths| !paths.is_empty()) {
            paths.clone()
        } else {
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
        }
    } else {
        files
    };

    // 3.以 Git diff 为主证据，并为长文件提供显式标记的头尾上下文。
    let identifiers = crate::plan_contract::acceptance_identifiers(
        acceptance_criteria.as_deref().unwrap_or_default(),
    )
    .into_iter()
    .collect::<Vec<_>>();
    let review_evidence = build_review_evidence(project_path, &files, &identifiers);
    // ===== 真测试：检测项目类型，执行对应的测试命令 =====
    let test_evidence = {
        let project_root = std::path::Path::new(project_path);

        // 优先检测自定义测试命令文件 .metheus-test
        let metheus_test_file = project_root.join(".metheus-test");
        if metheus_test_file.exists() {
            match std::fs::read_to_string(&metheus_test_file) {
                Ok(contents) => {
                    let cmd_line = contents.trim().to_string();
                    if cmd_line.is_empty() || cmd_line.starts_with('#') {
                        eprintln!("[check_subtask] .metheus-test 为空或注释，跳过");
                        AutomatedTestEvidence::not_configured(None)
                    } else {
                        let parts: Vec<&str> = cmd_line.split_whitespace().collect();
                        let cmd = parts[0];
                        let cmd_args = &parts[1..];
                        eprintln!("[check_subtask] 使用自定义测试命令: {}", cmd_line);
                        match run_test_command(cmd, cmd_args, project_path, 300) {
                            Ok((code, stdout, stderr)) => {
                                let summary = summarize_test_output(code, &stdout, &stderr);
                                let rendered =
                                    format_test_result("自定义测试", &cmd_line, code, &summary);
                                AutomatedTestEvidence::completed(&cmd_line, code, summary, rendered)
                            }
                            Err(e) => AutomatedTestEvidence::unavailable(
                                &cmd_line,
                                format!("自定义测试执行失败：{}", e),
                            ),
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[check_subtask] 读取 .metheus-test 失败: {}", e);
                    AutomatedTestEvidence::unavailable(
                        ".metheus-test",
                        format!("读取 .metheus-test 失败：{}", e),
                    )
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
                        AutomatedTestEvidence {
                            rendered: Some(format!(
                            "测试命令: {}\n状态: ⚠️ 未配置测试用例（{} 返回：{}）\n\n该项目未配置测试用例，请仅基于代码审查判定，不要将此视为测试失败。",
                            label, pm, stderr_preview
                            )),
                            command: label,
                            exit_code: Some(code),
                            output_summary: stderr_preview,
                            status: project::AutomatedTestStatus::NotConfigured,
                        }
                    } else {
                        let summary = summarize_test_output(code, &stdout, &stderr);
                        let rendered = format_test_result(&label, &label, code, &summary);
                        AutomatedTestEvidence::completed(&label, code, summary, rendered)
                    }
                }
                Err(e) => AutomatedTestEvidence::unavailable(
                    &label,
                    format!("{} test 执行失败（测试环境不可用）：{}", pm, e),
                ),
            }
        } else if project_root.join("Cargo.toml").exists() {
            // Rust 项目
            match run_test_command("cargo", &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    let rendered = format_test_result("cargo test", "cargo test", code, &summary);
                    AutomatedTestEvidence::completed("cargo test", code, summary, rendered)
                }
                Err(e) => AutomatedTestEvidence::unavailable(
                    "cargo test",
                    format!("cargo test 执行失败：{}", e),
                ),
            }
        } else if project_root.join("go.mod").exists() {
            // Go 项目
            match run_test_command("go", &["test", "./..."], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    let rendered = format_test_result("go test", "go test ./...", code, &summary);
                    AutomatedTestEvidence::completed("go test ./...", code, summary, rendered)
                }
                Err(e) => AutomatedTestEvidence::unavailable(
                    "go test ./...",
                    format!("go test 执行失败：{}", e),
                ),
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
                    let rendered = format_test_result(label, &full_cmd, code, &summary);
                    AutomatedTestEvidence::completed(&full_cmd, code, summary, rendered)
                }
                Err(e) => AutomatedTestEvidence::unavailable(
                    &full_cmd,
                    format!("{} 执行失败：{}", label, e),
                ),
            }
        } else if project_root.join("CMakeLists.txt").exists() {
            // C++ 项目
            match run_test_command("ctest", &[], project_path, 300) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    let rendered = format_test_result("ctest", "ctest", code, &summary);
                    AutomatedTestEvidence::completed("ctest", code, summary, rendered)
                }
                Err(e) => {
                    AutomatedTestEvidence::unavailable("ctest", format!("ctest 执行失败：{}", e))
                }
            }
        } else if project_root.join("pom.xml").exists() {
            // Java Maven
            match run_test_command("mvn", &["test"], project_path, 600) {
                Ok((code, stdout, stderr)) => {
                    let summary = summarize_test_output(code, &stdout, &stderr);
                    let rendered = format_test_result("mvn test", "mvn test", code, &summary);
                    AutomatedTestEvidence::completed("mvn test", code, summary, rendered)
                }
                Err(e) => AutomatedTestEvidence::unavailable(
                    "mvn test",
                    format!("mvn test 执行失败：{}", e),
                ),
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
                    let rendered = format_test_result("gradle test", "gradle test", code, &summary);
                    AutomatedTestEvidence::completed("gradle test", code, summary, rendered)
                }
                Err(e) => AutomatedTestEvidence::unavailable(
                    "gradle test",
                    format!("gradle test 执行失败：{}", e),
                ),
            }
        } else {
            eprintln!("[check_subtask] 未检测到已知测试框架，跳过真测试");
            AutomatedTestEvidence::not_configured(None)
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
    let acceptance_section = acceptance_criteria
        .as_ref()
        .filter(|items| !items.is_empty())
        .map(|items| format!("## 验收标准\n- {}\n\n", items.join("\n- ")))
        .unwrap_or_default();
    let execution_section = execution_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
        .map(|prompt| {
            let prompt: String = prompt.chars().take(2_000).collect();
            format!("## 执行提示\n{prompt}\n\n")
        })
        .unwrap_or_default();
    let authorized_section = authorized_paths
        .as_ref()
        .filter(|paths| !paths.is_empty())
        .map(|paths| format!("## 授权文件范围\n- {}\n\n", paths.join("\n- ")))
        .unwrap_or_default();
    let review_header = format!(
        "{}{}{}{}## 审查证据状态\n{}\n证据中出现省略标记时，不得据此断言省略区域中的函数、标签或实现不存在。\n\n",
        goal_section,
        acceptance_section,
        execution_section,
        authorized_section,
        review_evidence.summary,
    );
    let user_message = if let Some(ref test_result) = test_evidence.rendered {
        format!(
            "{}请检查以下代码改动。\n\n## 自动化测试结果\n项目自动化测试已执行，结果如下：\n\n{}\n\n---\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            review_header,
            test_result,
            files.len(),
            files.join("\n"),
            review_evidence.rendered
        )
    } else {
        format!(
            "{}请检查以下代码改动：\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            review_header,
            files.len(),
            files.join("\n"),
            review_evidence.rendered
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
            r#"{"passed": false, "issues": ["AI API 调用失败"], "suggestion": "", "warnings": []}"#
                .to_string()
        });
    // 解析 JSON 响应（带兜底）
    let mut test_result: project::TestResult =
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
                    ..Default::default()
                }
            }
        };
    let review_passed = test_result.passed;
    test_result.review_passed = review_passed;
    test_result.test_command = test_evidence.command;
    test_result.test_exit_code = test_evidence.exit_code;
    test_result.test_output_summary = test_evidence.output_summary;
    test_result.automated_test_status = test_evidence.status.clone();
    test_result.verification_kind = match test_evidence.status {
        project::AutomatedTestStatus::Passed | project::AutomatedTestStatus::Failed => {
            project::VerificationKind::AutomatedTestAndReview
        }
        project::AutomatedTestStatus::NotConfigured | project::AutomatedTestStatus::Unknown => {
            project::VerificationKind::CodeReviewOnly
        }
        project::AutomatedTestStatus::Unavailable => project::VerificationKind::Legacy,
    };
    test_result.review_evidence_status = review_evidence.status;
    test_result.review_evidence_summary = review_evidence.summary;

    match test_evidence.status {
        project::AutomatedTestStatus::Failed => {
            test_result.passed = false;
            if !test_result
                .issues
                .iter()
                .any(|issue| issue.contains("自动化测试失败"))
            {
                test_result.issues.push("自动化测试失败".to_string());
            }
        }
        project::AutomatedTestStatus::Unavailable => {
            test_result.passed = false;
            if !test_result
                .issues
                .iter()
                .any(|issue| issue.contains("测试环境不可用"))
            {
                test_result.issues.push("测试环境不可用".to_string());
            }
        }
        _ => {}
    }

    Ok(test_result)
}
