// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::env;
use std::fs;
mod project;
mod prompts;
mod constants;
mod api;
mod json_utils;
mod git_ops;
mod constitution;
mod diff;
mod test_runner;
mod commands;
mod pipeline;
mod executor;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::pipeline::PipelineState;

///获取项目文件的存储路径
fn get_project_path(name: &str) -> String {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.metheus/{}.json", home, name)
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

///保存项目数据到文件
pub(crate) fn save_project(project: &project::Project) -> Result<(), String> {
    //1. 确保 .metheus项目存在
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = format!("{}/.metheus", home);
    fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{}", e))?;
    //2.序列化为JSON
    let json = serde_json::to_string_pretty(project).map_err(|e| format!("序列化失败: {}", e))?;
    //3.写入文件
    let path = get_project_path(&project.name);
    fs::write(&path, json).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(())
}

/// 根据项目名字，从硬盘文件里加载项目数据
// 比如输入 "my_game"，就去 ~/.metheus/my_game.json 里读取，还原成 Project 对象
pub(crate) fn load_project(name: &str) -> Result<project::Project, String> {
    // 1. 根据名字生成文件路径（例如 "/home/张三/.metheus/my_game.json"）
    let path = get_project_path(name);

    // 2. 读取整个文件内容 → 得到一个 JSON 字符串
    //    如果文件不存在或无法读取，就返回错误
    let data = fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{}", e))?;

    // 3. 把 JSON 字符串解析成 Project 结构体
    //    如果格式不对（比如缺少必要字段），就返回错误
    let project = serde_json::from_str(&data).map_err(|e| format!("解析 JSON 失败：{}", e))?;

    // 4. 成功时，把 Project 对象装进 Ok 信封返回
    Ok(project)
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
            crate::commands::project_ops::persist_project,
            crate::commands::milestone::generate_milestones,
            crate::commands::milestone::regenerate_milestones_with_feedback,
            crate::commands::milestone::generate_mid_stages,
            crate::executor::execute_subtask,
            crate::test_runner::check_subtask,
            crate::commands::milestone::generate_next_prompt,
            crate::pipeline::start_execution,
            crate::pipeline::get_execution_status,
            crate::pipeline::pause_execution,
            crate::pipeline::resume_execution,
            crate::pipeline::stop_execution,
            crate::commands::project_ops::approve_mid_stage,
            crate::commands::project_ops::reject_mid_stage,
            crate::git_ops::git_save_node,
            crate::git_ops::git_save_subtask,
            crate::git_ops::git_rollback_to_mid_stage,
            crate::git_ops::git_rollback_to_subtask,
            crate::constitution::update_constitution,
            crate::constitution::compact_constitution,
            crate::constitution::read_constitution,
            crate::git_ops::get_git_tags_summary,
            crate::git_ops::get_current_diff,
            crate::commands::project_ops::validate_project_path,
            crate::commands::project_ops::get_project_files,
            crate::constitution::get_constitution_summary
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
