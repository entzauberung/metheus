// Copyright (C) 2026 Bruce Long
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// ...
// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::fs;
mod acceptance;
mod api;
mod automated_validation;
mod autopilot_failure;
mod autopilot_policy;
mod autopilot_runtime;
mod chat_runtime;
mod commands;
mod constants;
mod constitution;
mod constitution_context;
mod control_action;
mod control_action_executor;
mod control_scheduler;
mod control_snapshot;
mod cost_ledger;
mod diff;
mod engine;
mod git_ops;
mod human_action_policy;
mod json_utils;
mod managed_runtime;
mod pipeline;
mod plan_calibration;
mod plan_compiler;
mod plan_contract;
mod plan_deterministic_checks;
mod plan_scope;
mod project;
mod project_facts;
mod project_state_bus;
mod prompts;
mod provability;
mod quality_gate;
mod recovery;
mod recovery_checkpoint;
mod recovery_learning;
mod recovery_presentation;
mod review_evidence;
mod review_protocol;
mod runtime_snapshot;
mod settings;
mod snapshot;
mod task_aggregation;
mod task_compiler;
mod task_complexity;
mod task_contract;
mod task_control;
mod task_tree;
mod test_runner;
mod validator_contract;
mod validator_registry;
mod validators;
mod vision_review;
mod workflow_resolution;
mod workload_policy;
use crate::pipeline::PipelineState;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

static PROJECT_WRITE_LOCK: StdMutex<()> = StdMutex::new(());

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
    let path = project_data_path(&project.name)?;
    let persisted = {
        let _guard = PROJECT_WRITE_LOCK
            .lock()
            .map_err(|_| "项目写锁已损坏".to_string())?;
        write_project_to_path(project, &path)?
    };
    publish_persisted_project(&persisted);
    Ok(())
}

pub(crate) fn save_project_if_revision(
    project: &project::Project,
    expected_project_revision: u64,
    expected_tree_revision: u64,
) -> Result<(), String> {
    let path = project_data_path(&project.name)?;
    let persisted = {
        let _guard = PROJECT_WRITE_LOCK
            .lock()
            .map_err(|_| "项目写锁已损坏".to_string())?;
        let current = load_project_from_path(&path)?;
        if current.workflow_state.data_revision != expected_project_revision
            || current.task_control.tree_revision != expected_tree_revision
        {
            return Err("人工终态动作校验后项目状态已变化，拒绝旧动作".to_string());
        }
        write_project_to_path(project, &path)?
    };
    publish_persisted_project(&persisted);
    Ok(())
}

/// 在稳定的跨进程文件锁和进程内项目写锁下重新读取并修改项目。
/// 控制动作的认领、心跳、完成和清理必须使用本入口，避免两个应用进程同时认领。
pub(crate) fn mutate_project_for_control<T, F>(project_name: &str, mutate: F) -> Result<T, String>
where
    F: FnOnce(&mut project::Project) -> Result<(T, bool), String>,
{
    let path = project_data_path(project_name)?;
    let lock_path = path.with_extension("control-action.lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建控制锁目录失败：{}", error))?;
    }
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("打开控制动作事务锁失败：{}", error))?;
    lock_file
        .lock()
        .map_err(|error| format!("获取控制动作事务锁失败：{}", error))?;

    let (result, persisted) = {
        let _guard = PROJECT_WRITE_LOCK
            .lock()
            .map_err(|_| "项目写锁已损坏".to_string())?;
        let mut project = load_project_from_path(&path)?;
        let (result, changed) = mutate(&mut project)?;
        let persisted = if changed {
            Some(write_project_to_path(&project, &path)?)
        } else {
            None
        };
        (result, persisted)
    };
    drop(lock_file);
    if let Some(project) = persisted.as_ref() {
        publish_persisted_project(project);
    }
    Ok(result)
}

fn publish_persisted_project(persisted: &project::Project) {
    if let Err(error) = crate::project_state_bus::publish_project_state(persisted) {
        // The atomic replace already succeeded. Notification delivery is best effort and
        // must never turn a successful durable write into a reported save failure.
        eprintln!(
            "项目状态已保存，但状态修订通知发布失败（project={}）：{}",
            persisted.name, error
        );
    }
}

pub(crate) fn save_project_to_path(
    project: &project::Project,
    path: &std::path::Path,
) -> Result<(), String> {
    write_project_to_path(project, path).map(|_| ())
}

/// Pure atomic file write used by tests and explicit alternate paths.
///
/// The returned value is the exact merged value written to disk. This function never
/// publishes a project-state event; only `save_project` owns that side effect.
fn write_project_to_path(
    project: &project::Project,
    path: &std::path::Path,
) -> Result<project::Project, String> {
    //1. 确保目标目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{}", e))?;
    }
    // Preserve model calls recorded after this in-memory project snapshot was loaded.
    let mut value = project.clone();
    if let Ok(data) = fs::read_to_string(path) {
        if let Ok(on_disk) = serde_json::from_str::<project::Project>(&data) {
            value.cost_ledger.merge_from(&on_disk.cost_ledger);
        }
    }
    //2.序列化为JSON
    let json = serde_json::to_string_pretty(&value).map_err(|e| format!("序列化失败: {}", e))?;
    //3. 写入同目录临时文件
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &json).map_err(|e| format!("写入临时文件失败: {}", e))?;
    //4. 替换正式文件（原子 rename）
    fs::rename(&tmp_path, &path).map_err(|e| {
        // 清理临时文件
        let _ = fs::remove_file(&tmp_path);
        format!("替换项目文件失败: {}", e)
    })?;
    Ok(value)
}

/// 根据项目名字，从硬盘文件里加载项目数据
// 比如输入 "my_game"，就去 ~/.metheus/my_game.json 里读取，还原成 Project 对象
pub(crate) fn load_project(name: &str) -> Result<project::Project, String> {
    let path = project_data_path(name)?;
    load_project_from_path(&path)
}

pub(crate) fn load_project_from_path(path: &std::path::Path) -> Result<project::Project, String> {
    // 读取整个文件内容，再还原成 Project
    //    如果文件不存在或无法读取，就返回错误
    let data = fs::read_to_string(&path).map_err(|e| format!("读取文件失败：{}", e))?;

    // 3. 把 JSON 字符串解析成 Project 结构体
    //    如果格式不对（比如缺少必要字段），就返回错误
    let mut project: project::Project =
        serde_json::from_str(&data).map_err(|e| format!("解析 JSON 失败：{}", e))?;
    crate::task_control::hydrate_project(&mut project)?;

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
    pub(crate) pipeline_state: Arc<Mutex<Option<PipelineState>>>,
    pub(crate) autopilot_runtime: Arc<crate::autopilot_runtime::AutopilotRuntime>,
    pub(crate) managed_runtime: Arc<crate::managed_runtime::ManagedRuntime>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_env();
    if let Err(error) = crate::settings::initialize_settings() {
        eprintln!("初始化应用设置失败：{error}");
    }
    // 启动时清理上次异常退出遗留的孤儿进程
    crate::snapshot::cleanup_orphan_processes_at_startup();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            pipeline_state: Arc::new(Mutex::new(None)),
            autopilot_runtime: Arc::new(crate::autopilot_runtime::AutopilotRuntime::default()),
            managed_runtime: Arc::new(crate::managed_runtime::ManagedRuntime::default()),
        })
        .manage(crate::chat_runtime::ChatRuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            crate::commands::chat::greet,
            crate::commands::chat::send_message,
            crate::commands::project_ops::get_project,
            crate::commands::project_state::subscribe_project_state,
            crate::commands::project_state::unsubscribe_project_state,
            crate::runtime_snapshot::get_runtime_snapshot,
            crate::commands::runtime_mutations::select_milestone_runtime,
            crate::commands::runtime_mutations::select_mid_stage_runtime,
            crate::commands::runtime_mutations::generate_version_plan_runtime,
            crate::commands::runtime_mutations::approve_version_plan_runtime,
            crate::commands::runtime_mutations::reject_version_plan_runtime,
            crate::commands::runtime_mutations::enter_console_runtime,
            crate::commands::runtime_mutations::start_preflight_check_runtime,
            crate::commands::runtime_mutations::analyze_existing_project_runtime,
            crate::commands::runtime_mutations::approve_existing_baseline_runtime,
            crate::commands::runtime_mutations::update_execution_profile_runtime,
            crate::commands::runtime_mutations::migrate_project_workflow_runtime,
            crate::commands::runtime_mutations::reconcile_on_startup_runtime,
            crate::commands::runtime_mutations::return_to_discussion_runtime,
            crate::commands::runtime_mutations::resume_plan_approval_runtime,
            crate::commands::runtime_mutations::restart_discussion_from_approved_runtime,
            crate::commands::runtime_mutations::restart_checks_runtime,
            crate::commands::runtime_mutations::run_preflight_check_runtime,
            crate::commands::runtime_mutations::reconcile_managed_milestone_state_runtime,
            crate::commands::runtime_mutations::resolve_pause_decision_runtime,
            crate::commands::runtime_mutations::confirm_rollback_runtime,
            crate::commands::runtime_mutations::regenerate_execution_plan_runtime,
            crate::commands::runtime_mutations::generate_future_milestone_draft_runtime,
            crate::commands::runtime_mutations::approve_future_milestones_runtime,
            crate::commands::runtime_mutations::start_managed_flow_runtime,
            crate::commands::runtime_mutations::stop_managed_flow_runtime,
            crate::commands::runtime_mutations::toggle_autopilot_runtime,
            crate::commands::runtime_mutations::autopilot_pause_runtime,
            crate::commands::runtime_mutations::autopilot_resume_runtime,
            crate::commands::runtime_mutations::request_in_stop_runtime,
            crate::commands::runtime_mutations::request_ed_stop_runtime,
            crate::commands::runtime_mutations::confirm_subtask_result_runtime,
            crate::commands::runtime_mutations::retry_git_confirmation_runtime,
            crate::commands::runtime_mutations::reject_subtask_result_runtime,
            crate::commands::runtime_mutations::run_error_recovery_runtime,
            crate::commands::runtime_mutations::acknowledge_execution_recovery_runtime,
            crate::commands::runtime_mutations::resolve_human_recovery_runtime,
            crate::commands::runtime_mutations::approve_milestone_outcome_runtime,
            crate::commands::runtime_mutations::update_human_review_policy_runtime,
            crate::commands::runtime_mutations::summarize_milestone_runtime,
            crate::commands::runtime_mutations::generate_milestone_draft_runtime,
            crate::commands::runtime_mutations::regenerate_milestone_draft_runtime,
            crate::commands::runtime_mutations::check_milestone_draft_runtime,
            crate::commands::runtime_mutations::approve_milestone_draft_runtime,
            crate::commands::runtime_mutations::continue_current_milestone_runtime,
            crate::commands::runtime_mutations::generate_mid_stage_draft_runtime,
            crate::commands::runtime_mutations::regenerate_mid_stage_draft_runtime,
            crate::commands::runtime_mutations::check_mid_stage_draft_runtime,
            crate::commands::runtime_mutations::approve_mid_stage_draft_runtime,
            crate::commands::runtime_mutations::generate_execution_plan_runtime,
            crate::commands::runtime_mutations::check_stage_plan_runtime,
            crate::commands::runtime_mutations::approve_stage_plan_runtime,
            crate::commands::runtime_mutations::prepare_execution_workspace_runtime,
            crate::commands::runtime_mutations::refresh_execution_workspace_runtime,
            crate::commands::runtime_mutations::execute_current_subtask_runtime,
            crate::commands::runtime_mutations::pause_managed_flow_runtime,
            crate::commands::runtime_mutations::resume_managed_flow_runtime,
            crate::commands::project_ops::check_engine_health,
            crate::commands::project_ops::verify_engine_authentication,
            crate::commands::project_ops::update_execution_profile,
            crate::commands::settings::get_app_settings,
            crate::commands::settings::update_app_settings,
            crate::commands::settings::set_api_secret,
            crate::commands::settings::clear_api_secret,
            crate::commands::settings::test_model_connection,
            crate::commands::settings::test_grok_build_runtime,
            crate::commands::chat::chat_with_role,
            crate::commands::chat::chat_with_role_stream,
            crate::commands::chat::regenerate_chat_reply_stream,
            crate::commands::chat::chat_with_role_runtime,
            crate::commands::chat::chat_with_role_stream_runtime,
            crate::commands::chat::regenerate_chat_reply_stream_runtime,
            crate::commands::chat::cancel_chat_stream,
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
            crate::commands::milestone::continue_current_milestone,
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
            crate::pipeline::retry_git_confirmation,
            crate::pipeline::reject_subtask_result,
            crate::pipeline::retry_current_subtask,
            crate::pipeline::get_execution_workspace_status,
            crate::pipeline::refresh_execution_workspace,
            crate::pipeline::prepare_execution_workspace,
            crate::pipeline::get_execution_status,
            crate::pipeline::request_in_stop,
            crate::pipeline::request_ed_stop,
            crate::pipeline::resolve_pause_decision,
            crate::pipeline::preview_rollback_impact,
            crate::pipeline::preview_execution_recovery_impact,
            crate::pipeline::confirm_rollback,
            crate::pipeline::reconcile_on_startup,
            crate::pipeline::acknowledge_execution_recovery,
            crate::recovery::run_error_recovery,
            crate::recovery::resolve_human_recovery,
            crate::plan_calibration::calibrate_next_subtask_command,
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
            crate::commands::workflow::wait_managed_flow_for_human,
            crate::commands::workflow::resume_managed_flow,
            crate::commands::workflow::stop_managed_flow,
            crate::commands::workflow::reconcile_managed_milestone_state,
            crate::commands::workflow::start_preflight_check,
            crate::commands::workflow::return_to_discussion,
            crate::commands::workflow::resume_plan_approval,
            crate::commands::workflow::restart_discussion_from_approved,
            crate::commands::workflow::restart_checks,
            crate::commands::task_control::get_task_control_snapshot,
            crate::commands::task_control::set_task_control_mode,
            crate::commands::task_control::apply_task_control_action,
            crate::commands::task_control::set_task_control_mode_runtime,
            crate::commands::task_control::apply_task_control_action_runtime,
            crate::commands::project_ops::initialize_project_entry,
            crate::commands::project_ops::validate_project_path,
            crate::commands::project_ops::get_project_files,
            crate::commands::project_ops::read_project_file_preview,
            crate::constitution::get_constitution_summary,
            crate::constitution::get_constitution_change_history,
            crate::snapshot::save_snapshot_event,
            crate::snapshot::restore_snapshot
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_execution_contract_loads_v1_snapshot_without_profile_and_preserves_task_facts() {
        let path = std::env::temp_dir().join(format!(
            "metheus-v1-contract-load-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut project = project::Project::new("legacy-contract-load");
        project.workload_profile = None;
        project.milestones.push(project::Milestone {
            id: "m".to_string(),
            title: "Legacy milestone".to_string(),
            status: project::MilestoneStatus::InProgress,
            mode: project::StageMode::Quick,
            subtasks: vec![project::Subtask {
                id: "legacy-task".to_string(),
                title: "Legacy task".to_string(),
                status: project::SubtaskStatus::AcceptedDeviation,
                acceptance_criteria: vec!["保留历史验收内容".to_string()],
                confirmation_notes: Some("保留人工决定".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut value = serde_json::to_value(project).unwrap();
        value["milestones"][0]["subtasks"][0]["contract_snapshot"] = serde_json::json!({
            "version": "task-contract-v1",
            "task_id": "legacy-task"
        });
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let loaded = load_project_from_path(&path);
        let _ = fs::remove_file(&path);
        let loaded = loaded.unwrap();
        let task = &loaded.milestones[0].subtasks[0];
        assert_eq!(task.status, project::SubtaskStatus::AcceptedDeviation);
        assert_eq!(task.acceptance_criteria, vec!["保留历史验收内容"]);
        assert_eq!(task.confirmation_notes.as_deref(), Some("保留人工决定"));
        assert!(task.contract_snapshot.is_none());
        assert_eq!(
            loaded.task_control.takeover_capability_status,
            crate::task_control::TakeoverCapabilityStatus::Unavailable
        );
        let error = crate::workload_policy::current_profile(&loaded).unwrap_err();
        assert!(error.contains("目标完整性检查"));
        assert!(error.contains("重新"));
    }
}
