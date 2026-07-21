// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::fs;
mod api;
mod commands;
mod constants;
mod constitution;
mod constitution_context;
mod diff;
mod executor;
mod git_ops;
mod json_utils;
mod pipeline;
mod plan_contract;
mod project;
mod prompts;
mod snapshot;
mod test_runner;
use crate::pipeline::PipelineState;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 获取项目数据文件的统一存储路径
///
/// 返回 `~/.metheus/{project_id}.json`，使用 `dirs::home_dir()` 跨平台获取家目录。
/// 所有读写 project.json 的模块（lib、git_ops、pipeline）统一调用此函数。
pub(crate) fn project_data_path(project_id: &str) -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户家目录路径".to_string())?;
    Ok(home.join(".metheus").join(format!("{}.json", project_id)))
}

/// 校验项目路径：存在性、目录类型、git 仓库
pub(crate) fn check_project_path(path: &str) -> project::PathValidationResult {
    let p = std::path::Path::new(path);
    let exists = p.exists();
    let is_directory = exists && p.is_dir();
    // 兼容 worktree：.git 可能是文件而非目录
    let is_git_repo = is_directory && p.join(".git").exists();

    let mut errors: Vec<&str> = Vec::new();
    if !exists {
        errors.push("路径不存在");
    } else if !is_directory {
        errors.push("路径不是目录");
    }

    project::PathValidationResult {
        is_valid: exists && is_directory,
        exists,
        is_directory,
        is_git_repo,
        error_message: if errors.is_empty() {
            String::new()
        } else {
            errors.join("；")
        },
    }
}

///保存项目数据到文件（原子写入：先写临时文件，再替换正式文件）
pub(crate) fn save_project(project: &project::Project) -> Result<(), String> {
    //1. 确保 .metheus 目录存在
    let path = project_data_path(&project.name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{}", e))?;
    }
    //2.序列化为JSON
    let json = serde_json::to_string_pretty(project).map_err(|e| format!("序列化失败: {}", e))?;
    //3. 写入同目录临时文件
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &json).map_err(|e| format!("写入临时文件失败: {}", e))?;
    //4. 替换正式文件（原子 rename）
    fs::rename(&tmp_path, &path).map_err(|e| {
        // 清理临时文件
        let _ = fs::remove_file(&tmp_path);
        format!("替换项目文件失败: {}", e)
    })?;
    Ok(())
}

/// 根据项目名字，从硬盘文件里加载项目数据
// 比如输入 "my_game"，就去 ~/.metheus/my_game.json 里读取，还原成 Project 对象
pub(crate) fn load_project(name: &str) -> Result<project::Project, String> {
    // 1. 根据名字生成文件路径（例如 "/home/张三/.metheus/my_game.json"）
    let path = project_data_path(name)?;

    // 2. 读取整个文件内容 → 得到一个 JSON 字符串
    //    如果文件不存在或无法读取，就返回错误
    let data = fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{}", e))?;

    // 3. 把 JSON 字符串解析成 Project 结构体
    //    如果格式不对（比如缺少必要字段），就返回错误
    let project = serde_json::from_str(&data).map_err(|e| format!("解析 JSON 失败：{}", e))?;

    // 4. 成功时，把 Project 对象装进 Ok 信封返回
    Ok(project)
}

/// Persist a project and return the exact value that can be read back from disk.
pub(crate) fn save_and_reload_project(
    project: &project::Project,
) -> Result<project::Project, String> {
    let project_name = project.name.clone();
    save_project(project)?;
    load_project(&project_name)
        .map_err(|error| format!("项目已保存，但重新读取磁盘最终状态失败：{}", error))
}

fn load_env() {
    dotenvy::dotenv().ok();
}

pub struct AppState {
    pub pipeline_state: Arc<Mutex<Option<PipelineState>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_env();
    // 启动时清理上次异常退出遗留的孤儿进程
    crate::snapshot::cleanup_orphan_processes_at_startup();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            pipeline_state: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::chat::greet,
            crate::commands::chat::send_message,
            crate::commands::project_ops::get_project,
            crate::commands::chat::chat_with_role,
            crate::commands::plan::generate_version_plan,
            crate::commands::plan::approve_version_plan,
            crate::commands::plan::reject_version_plan,
            crate::commands::plan::enter_console,
            crate::commands::milestone::generate_milestone_draft,
            crate::commands::milestone::regenerate_milestone_draft,
            crate::commands::milestone::check_milestone_draft,
            crate::commands::milestone::approve_milestone_draft,
            crate::commands::milestone::select_milestone,
            crate::commands::milestone::generate_mid_stage_draft,
            crate::commands::milestone::regenerate_mid_stage_draft,
            crate::commands::milestone::check_mid_stage_draft,
            crate::commands::milestone::approve_mid_stage_draft,
            crate::commands::milestone::select_mid_stage,
            crate::commands::milestone::generate_execution_plan,
            crate::commands::milestone::regenerate_execution_plan,
            crate::commands::milestone::check_stage_plan,
            crate::commands::milestone::approve_stage_plan,
            crate::commands::milestone::enter_milestone_review,
            crate::commands::milestone::approve_milestone_outcome,
            crate::commands::milestone::suggest_rollback_checkpoint,
            crate::commands::milestone::generate_future_milestone_draft,
            crate::commands::milestone::approve_future_milestones,
            crate::test_runner::check_subtask,
            crate::commands::milestone::summarize_milestone,
            crate::pipeline::execute_current_subtask,
            crate::pipeline::confirm_subtask_result,
            crate::pipeline::reject_subtask_result,
            crate::pipeline::retry_current_subtask,
            crate::pipeline::get_execution_workspace_status,
            crate::pipeline::prepare_execution_workspace,
            crate::pipeline::get_execution_status,
            crate::pipeline::request_in_stop,
            crate::pipeline::request_ed_stop,
            crate::pipeline::resolve_pause_decision,
            crate::pipeline::preview_rollback_impact,
            crate::pipeline::confirm_rollback,
            crate::pipeline::reconcile_on_startup,
            crate::pipeline::acknowledge_execution_recovery,
            crate::commands::project_ops::approve_mid_stage,
            crate::commands::project_ops::reject_mid_stage,
            crate::constitution::update_constitution,
            crate::constitution::compact_constitution,
            crate::constitution::read_constitution,
            crate::git_ops::get_git_tags_summary,
            crate::git_ops::get_current_diff,
            crate::git_ops::get_change_history,
            crate::commands::project_analysis::analyze_existing_project,
            crate::commands::project_analysis::scan_existing_project,
            crate::commands::project_analysis::generate_existing_baseline,
            crate::commands::project_analysis::approve_existing_baseline,
            crate::commands::checks::run_preflight_check,
            crate::commands::workflow::transition_workflow,
            crate::commands::workflow::migrate_project_workflow,
            crate::commands::workflow::toggle_autopilot,
            crate::commands::workflow::autopilot_pause,
            crate::commands::workflow::autopilot_resume,
            crate::commands::workflow::autopilot_mark_error,
            crate::commands::workflow::autopilot_next_step,
            crate::commands::workflow::start_managed_flow,
            crate::commands::workflow::managed_next_step,
            crate::commands::workflow::pause_managed_flow,
            crate::commands::workflow::resume_managed_flow,
            crate::commands::workflow::stop_managed_flow,
            crate::commands::workflow::start_preflight_check,
            crate::commands::workflow::return_to_discussion,
            crate::commands::workflow::resume_plan_approval,
            crate::commands::workflow::restart_discussion_from_approved,
            crate::commands::workflow::restart_checks,
            crate::commands::project_ops::initialize_project_entry,
            crate::commands::project_ops::validate_project_path,
            crate::commands::project_ops::get_project_files,
            crate::constitution::get_constitution_summary,
            crate::constitution::get_constitution_change_history,
            crate::snapshot::save_snapshot_event,
            crate::snapshot::restore_snapshot
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
