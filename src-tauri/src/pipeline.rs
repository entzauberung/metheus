use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};
use crate::project;
use crate::AppState;
use crate::check_project_path;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineState {
    pub mid_stage_id: String,
    pub status: PipelineStatus,
    pub current_subtask_index: usize,
    pub total_subtasks: usize,
    pub subtask_statuses: Vec<SubtaskStatusItem>,
    pub current_log: String,
    pub last_error: Option<String>,
}


/// 启动后台流水线执行一组子任务
#[tauri::command]
pub(crate) async fn start_execution(
    state: tauri::State<'_, AppState>,
    project_id: String,
    project_path: String,
    mid_stage_id: String,
    mid_stage_title: String,
    mid_stage_description: String,
    subtasks_json: String,
) -> Result<(), String> {
    // 前置校验：项目路径有效性
    let path_check = check_project_path(&project_path);
    if !path_check.is_valid {
        return Err(format!(
            "项目目录无效，无法启动执行：{}",
            path_check.error_message
        ));
    }
    // 自动初始化 git 仓库（路径有效但不是 git 仓库时）
    if !path_check.is_git_repo {
        let init = std::process::Command::new("git")
            .args(["init"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("{}：git 命令不可用 — {}", crate::constants::GIT_INIT_FAILED, e))?;
        if !init.status.success() {
            let stderr = String::from_utf8_lossy(&init.stderr);
            let truncated: String = stderr.chars().take(200).collect();
            return Err(format!("{}：{}", crate::constants::GIT_INIT_FAILED, truncated));
        }
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git add 失败：{}", e))?;
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", crate::constants::GIT_AUTO_INIT_COMMIT_MSG])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git commit 失败：{}", e))?;
    }
    // 解析子任务列表：把 subtasks_json 转成 Rust 结构体 Vec<Subtask>
    let subtasks: Vec<project::Subtask> =
        serde_json::from_str(&subtasks_json).map_err(|e| format!("解析小阶段列表失败：{}", e))?;
    if subtasks.is_empty() {
        return Err("子任务列表为空，请先生成执行计划".to_string());
    }
    let pipeline_state = state.pipeline_state.clone();
    // 初始化状态 在全局共享状态中创建一个 PipelineState，
    // 记录当前阶段 ID、总任务数、每个子任务的状态（等待/执行中/成功/失败）、当前日志等
    {
        let mut guard = pipeline_state.lock().await;
        *guard = Some(PipelineState {
            mid_stage_id: mid_stage_id.clone(),
            status: PipelineStatus::Running,
            current_subtask_index: 0,
            total_subtasks: subtasks.len(),
            subtask_statuses: subtasks
                .iter()
                .map(|s| SubtaskStatusItem {
                    subtask_id: s.id.clone(),
                    title: s.title.clone(),
                    status: "waiting".to_string(),
                    test_result: None,
                    retry_count: 0,
                })
                .collect(),
            current_log: "🚀 流水线已启动".to_string(),
            last_error: None,
        });
    }
    // 启动后台任务：
    // 用 tokio::spawn 启动一个异步任务，调用 execute_mid_stage_pipeline 真正去执行这些子任务
    tokio::spawn(async move {
        let result = execute_mid_stage_pipeline(
            project_id.clone(),
            mid_stage_id.clone(),
            project_path,
            subtasks,
            mid_stage_title,
            mid_stage_description,
            pipeline_state.clone(),
        )
        .await;
        if let Err(e) = result {
            let mut guard = pipeline_state.lock().await;
            // 捕获失败：如果后台任务执行失败，会将全局状态中的流水线标记为 Failed，并记录错误日志
            if let Some(s) = guard.as_mut() {
                s.status = PipelineStatus::Failed;
                s.last_error = Some(e.clone());
                s.current_log = format!("❌ 流水线失败: {}", e);
            }
        }
    });
    // 不等待结果直接返回：
    // 函数立即返回 Ok(())，后台任务继续运行。这样前端调用后不会卡住
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_execution_status(
    state: tauri::State<'_, AppState>,
) -> Result<Option<PipelineState>, String> {
    let guard = state.pipeline_state.lock().await;
    Ok(guard.clone())
}

/// 暂停流水线执行
#[tauri::command]
pub(crate) async fn pause_execution(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.pipeline_state.lock().await;
    match guard.as_mut() {
        Some(s) if s.status == PipelineStatus::Running => {
            s.status = PipelineStatus::Paused;
            s.current_log = "⏸ 已暂停".to_string();
            Ok(())
        }
        _ => Err("当前没有正在执行的流水线".to_string()),
    }
}


/// 恢复流水线执行
#[tauri::command]
pub(crate) async fn resume_execution(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.pipeline_state.lock().await;
    match guard.as_mut() {
        Some(s) if s.status == PipelineStatus::Paused => {
            s.status = PipelineStatus::Running;
            s.current_log = "▶ 已恢复".to_string();
            Ok(())
        }
        _ => Err("当前没有已暂停的流水线".to_string()),
    }
}


/// 停止流水线执行
#[tauri::command]
pub(crate) async fn stop_execution(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.pipeline_state.lock().await;
    match guard.as_mut() {
        Some(s) => {
            s.status = PipelineStatus::Failed;
            s.last_error = Some("用户手动停止".to_string());
            s.current_log = "⏹ 已停止".to_string();
            Ok(())
        }
        None => {
            // 没有活跃执行，幂等返回成功
            Ok(())
        }
    }
}


/// 这个函数就是一条自动流水线：
/// 逐个执行子任务，每个子任务最多重试 3 次，期间可以暂停/恢复，并实时更新进度面板。
/// 1. 初始化（准备空账本）
///   ↓
/// 2. 循环每个子任务（洗菜 → 切菜 → 炒菜）
///   ├─ 2.1 更新状态：开始做这道菜
///   ├─ 2.2 内层重试循环（最多 3 次）
///   │    ├─ 暂停检查（如果老板喊停，就等待恢复）
///   │    ├─ 生成提示词（告诉厨师下一步做什么）
///   │    ├─ 执行子任务（厨师做菜）
///   │    ├─ 运行测试（质检员尝菜）
///   │    ├─ 如果通过 → 记录成功，跳出重试循环
///   │    └─ 如果失败 → 重试次数+1，继续重试
///   └─ 2.3 更新状态：这道菜完成
///   ↓
/// 3. 全部子任务完成 → 更新最终状态为"完成"
/// 按顺序执行一组子任务（subtasks），每个子任务可能重试最多 3 次，全部通过后标记中阶段完成；
/// 过程中实时更新全局状态（进度、日志、测试结果），并支持暂停/恢复。
pub(crate) async fn execute_mid_stage_pipeline(
    project_id: String,
    mid_stage_id: String,
    project_path: String,
    mut subtasks: Vec<project::Subtask>,
    mid_stage_title: String,
    mid_stage_description: String,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<(), String> {
    // 初始化变量
    let mut previous_result = String::new();
    let mut previous_title = String::new();
    let mut file_changes: Vec<String> = vec![];
    let mut last_test_result = String::new();
    let mut mid_stage_version = String::new();

    // 提前从 project 文件中提取 mid_stage_version（避免只在函数末尾获取）
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let project_file = std::path::Path::new(&home)
            .join(".metheus")
            .join(format!("{}.json", project_id));
        if let Ok(content) = std::fs::read_to_string(&project_file) {
            if let Ok(project) = serde_json::from_str::<project::Project>(&content) {
                for milestone in &project.milestones {
                    for mid_stage in &milestone.mid_stages {
                        if mid_stage.id == mid_stage_id {
                            mid_stage_version = mid_stage.version.clone();
                            break;
                        }
                    }
                    if !mid_stage_version.is_empty() {
                        break;
                    }
                }
            }
        }
    }

    for i in 0..subtasks.len() {
        let subtask_title = subtasks[i].title.clone();
        let subtask_id = subtasks[i].id.clone();
        let mut retry_count = 0u32;
        let max_retries = 3u32;
        // 更新状态
        // 标记当前子任务为 "executing"，更新 current_log
        {
            let mut guard = state.lock().await;
            if let Some(s) = guard.as_mut() {
                s.current_subtask_index = i;
                if i > 0 {
                    s.subtask_statuses[i - 1].status = "passed".to_string();
                }
                s.subtask_statuses[i].status = "executing".to_string();
                s.current_log =
                    format!("▶ 执行中 ({}/{})：{}", i + 1, subtasks.len(), subtask_title);
            }
        }
        while retry_count < max_retries {
            // 暂停检查
            // 如果全局状态变成 Paused，就循环等待直到恢复
            {
                let guard = state.lock().await;
                if let Some(s) = guard.as_ref() {
                    if s.status == PipelineStatus::Paused {
                        drop(guard);
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            let guard2 = state.lock().await;
                            if let Some(s2) = guard2.as_ref() {
                                if s2.status != PipelineStatus::Paused {
                                    // 恢复后立即检查：如果被标记为 Failed，直接终止流水线
                                    if s2.status == PipelineStatus::Failed {
                                        return Err("流水线已被取消".to_string());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            // 生成 prompt：根据上一步结果、文件变更、测试结果等，调用 generate_next_prompt 得到下一步的标题和指令
            let generated = crate::commands::milestone::generate_next_prompt(
                mid_stage_title.clone(),
                mid_stage_description.clone(),
                previous_title.clone(),
                previous_result.clone(),
                file_changes.clone(),
                last_test_result.clone(),
                retry_count > 0,
                if retry_count > 0 {
                    last_test_result.clone()
                } else {
                    String::new()
                },
            )
            .await?;
            // 更新日志
            {
                let mut guard = state.lock().await;
                if let Some(s) = guard.as_mut() {
                    s.current_log = format!("⚙️ {}", generated.title);
                }
            }
            // 执行子任务（可被暂停中断）
            let exec_result = match crate::executor::execute_subtask_inner(
                &project_path,
                &generated.prompt,
                &subtask_id,
                state.clone(),
            )
            .await
            {
                Ok(r) => r,
                Err(project::SubTaskError::UserPaused) => {
                    // 用户暂停：Claude Code 已被 kill，进入等待恢复循环
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        let guard = state.lock().await;
                        if let Some(s) = guard.as_ref() {
                            if s.status != PipelineStatus::Paused {
                                // 已恢复（或已取消）
                                if s.status == PipelineStatus::Failed {
                                    return Err("流水线已被取消".to_string());
                                }
                                break;
                            }
                        }
                    }
                    // 恢复后重新执行当前子任务（不增加 retry_count）
                    continue;
                }
                Err(e) => {
                    let msg = match e {
                        project::SubTaskError::ExecutionFailed { message } => message,
                        project::SubTaskError::Timeout => "执行超时".to_string(),
                        _ => format!("{:?}", e),
                    };
                    return Err(format!("Claude Code 执行失败：{}", msg));
                }
            };
            subtasks[i].execution_result = Some(exec_result.clone());
            file_changes = exec_result.file_changes.clone();
            // 记录执行结果到日志
            {
                let mut guard = state.lock().await;
                if let Some(s) = guard.as_mut() {
                    if exec_result.success {
                        s.current_log = format!(
                            "✅ 完成: {} (变更 {} 个文件)",
                            subtask_title,
                            file_changes.len()
                        );
                    } else {
                        s.current_log = format!(
                            "❌ 失败: {} — {}",
                            subtask_title,
                            exec_result.error_log.chars().take(100).collect::<String>()
                        );
                    }
                }
            }
            // Claude Code 进程失败 -> 不进入重试循环，直接中止
            if !exec_result.success {
                return Err(format!("Claude Code 执行失败：{}", exec_result.error_log));
            }
            // 运行测试
            // 调用 check_subtask，得到 test.passed
            {
                let mut guard = state.lock().await;
                if let Some(s) = guard.as_mut() {
                    s.subtask_statuses[i].status = "testing".to_string();
                    s.current_log = format!("🔍 测试: {}", subtask_title);
                }
            }
            // 检查
            let test = match crate::test_runner::check_subtask(
                &project_path,
                &generated.prompt,
                &subtask_id,
                &subtask_title,
                &mid_stage_id,
            )
            .await
            {
                Ok(t) => t,
                Err(err) => project::TestResult {
                    passed: false,
                    issues: vec![format!("测试服务不可用: {}", err)],
                    suggestion: "请手动检查".to_string(),
                    warnings: vec![],
                },
            };
            last_test_result = if test.passed {
                "通过".to_string()
            } else if test.issues.is_empty() {
                "不通过（测试工程师未提供具体问题）".to_string()
            } else {
                let issues_text = test
                    .issues
                    .iter()
                    .map(|issue| format!("- {}", issue))
                    .collect::<Vec<_>>()
                    .join("\n");
                let full = format!("不通过。具体问题：\n{}", issues_text);
                // 防止 retry_reason 过长，截断到 1000 字符
                if full.chars().count() > 1000 {
                    format!("{}…（已截断）", full.chars().take(1000).collect::<String>())
                } else {
                    full
                }
            };
            if test.passed {
                {
                    let mut guard = state.lock().await;
                    if let Some(s) = guard.as_mut() {
                        s.subtask_statuses[i].status = "passed".to_string();
                        s.subtask_statuses[i].test_result = Some(test.clone());
                        s.current_log = format!("✅ 通过: {}", subtask_title);
                    }
                }
                previous_result = "通过".to_string();
                previous_title = subtask_title.clone();
                subtasks[i].test_result = Some(test);
                subtasks[i].retry_count = retry_count;

                // === 宪法更新链 ===
                let mut constitution_updated = false;
                let mut old_constitution = String::new();
                // 步骤 1：获取 git diff
                match std::process::Command::new("git")
                    .args(["diff", "HEAD"])
                    .current_dir(&project_path)
                    .output()
                {
                    Ok(output) => {
                        let diff_stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        // 步骤 2：提取变更摘要
                        let diff_summary = crate::diff::extract_diff_summary(&diff_stdout);
                        // 步骤 3：检查是否有实际变更
                        if diff_summary.new_files.is_empty()
                            && diff_summary.modified_files.is_empty()
                            && diff_summary.deleted_files.is_empty()
                            && diff_summary.new_functions.is_empty()
                            && diff_summary.modified_functions.is_empty()
                            && diff_summary.deleted_functions.is_empty()
                            && diff_summary.changed_dependencies.is_empty()
                        {
                            eprintln!("[constitution] 宪法更新跳过（无变更）");
                        } else {
                            // 步骤 4：读取当前 CONSTITUTION.md
                            let constitution_path =
                                std::path::Path::new(&project_path).join("CONSTITUTION.md");
                            let constitution_content =
                                std::fs::read_to_string(&constitution_path).unwrap_or_default();
                            old_constitution = constitution_content.clone();
                            // 步骤 5：调用 update_constitution
                            match crate::constitution::update_constitution(constitution_content.clone(), diff_summary)
                                .await
                            {
                                Ok(updated_content) => {
                                    // 步骤 6：写回 CONSTITUTION.md
                                    if let Err(e) =
                                        std::fs::write(&constitution_path, &updated_content)
                                    {
                                        eprintln!(
                                            "[constitution] 写入 CONSTITUTION.md 失败：{}",
                                            e
                                        );
                                    } else {
                                        constitution_updated = true;
                                        // 步骤 6b：检查是否需要剪枝
                                        // 提取第 2 部分，超过阈值则触发 compact_constitution
                                        if let Some(part2_start) =
                                            updated_content.find("## 第 2 部分")
                                        {
                                            let part2 = &updated_content[part2_start..];
                                            if crate::constitution::estimate_tokens(part2) > crate::constants::COMPACTION_TRIGGER_TOKENS {
                                                match crate::constitution::compact_constitution(updated_content.clone())
                                                    .await
                                                {
                                                    Ok(compacted) => {
                                                        if let Err(e) = std::fs::write(
                                                            &constitution_path,
                                                            &compacted,
                                                        ) {
                                                            eprintln!(
                                                                "[constitution] 写入剪枝后宪法失败：{}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!(
                                                            "[constitution] 宪法剪枝失败，保留膨胀版本：{}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[constitution] 宪法更新失败：{}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[constitution] git diff 失败，跳过宪法更新：{}", e);
                    }
                }

                // === git_save_subtask ===
                match crate::git_ops::git_save_subtask(
                    project_path.clone(),
                    (i + 1) as u32,
                    mid_stage_version.clone(),
                    subtask_title.clone(),
                )
                .await
                {
                    Ok(tag_name) => {
                        subtasks[i].auto_tag = Some(tag_name);
                    }
                    Err(e) => {
                        eprintln!("[constitution] git_save_subtask 失败：{}", e);
                        // 如果宪法在此次流水线中被更新过，回退宪法到更新前的内容
                        if constitution_updated {
                            let constitution_path =
                                std::path::Path::new(&project_path).join("CONSTITUTION.md");
                            if let Err(e2) = std::fs::write(&constitution_path, &old_constitution) {
                                eprintln!(
                                    "[constitution] 宪法回退写入也失败，宪法可能处于不一致状态：{}",
                                    e2
                                );
                            } else {
                                eprintln!(
                                    "[constitution] git_save_subtask 失败，宪法已回退到更新前状态"
                                );
                            }
                        }
                    }
                }

                break;
            } else {
                retry_count += 1;
                subtasks[i].retry_count = retry_count;
                {
                    let mut guard = state.lock().await;
                    if let Some(s) = guard.as_mut() {
                        s.subtask_statuses[i].status = "retrying".to_string();
                        s.subtask_statuses[i].retry_count = retry_count;
                        s.current_log =
                            format!("🔄 重试 {}/{}: {}", retry_count, max_retries, subtask_title);
                    }
                }
                if retry_count >= max_retries {
                    return Err(format!(
                        "小阶段「{}」重试 {} 次仍未通过",
                        subtask_title, max_retries
                    ));
                }
            }
        }
    }
    // 全部完成
    // 更新状态为 "passed"，记录测试结果，break 出 while 循环
    {
        let mut guard = state.lock().await;
        if let Some(s) = guard.as_mut() {
            s.status = PipelineStatus::Completed;
            if let Some(last) = s.subtask_statuses.last_mut() {
                last.status = "passed".to_string();
            }
            s.current_log = "✅ 所有小阶段执行完成！".to_string();
        }
    }
    // 写回 project 文件
    // 流水线跑完后，找到项目文件里对应的那个"中阶段"（MidStage），
    // 把每个子任务的执行结果、测试结果、重试次数填进去，
    // 然后把中阶段状态改成 "completed"，最后保存文件
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let project_file = std::path::Path::new(&home)
        .join(".metheus")
        .join(format!("{}.json", project_id));
    if let Ok(content) = std::fs::read_to_string(&project_file) {
        if let Ok(mut project) = serde_json::from_str::<project::Project>(&content) {
            // 找到对应的 MidStage，更新结果
            for milestone in &mut project.milestones {
                for mid_stage in &mut milestone.mid_stages {
                    if mid_stage.id == mid_stage_id {
                        mid_stage_version = mid_stage.version.clone();
                        for (i, subtask) in mid_stage.subtasks.iter_mut().enumerate() {
                            if i < subtasks.len() {
                                subtask.execution_result = subtasks[i].execution_result.clone();
                                subtask.test_result = subtasks[i].test_result.clone();
                                subtask.retry_count = subtasks[i].retry_count;
                                subtask.auto_tag = subtasks[i].auto_tag.clone();
                            }
                        }
                        mid_stage.status = project::MidStageStatus::Completed;
                        break;
                    }
                }
            }
            // 保存
            if let Ok(json) = serde_json::to_string_pretty(&project) {
                let _ = std::fs::write(&project_file, json);
            }
        }
    }
    // === 全部小阶段完成，自动 Git 存档 ===
    let tag_name = format!("metheus/{}", mid_stage_version);
    crate::git_ops::git_save_node(
        project_path.to_string(),
        mid_stage_version.clone(),
        mid_stage_title.to_string(),
    )
    .await?;
    crate::git_ops::save_tag_to_mid_stage(&project_id, &mid_stage_id, &tag_name)?;
    Ok(())
}
