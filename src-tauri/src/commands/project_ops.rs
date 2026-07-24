use crate::{project, AppState};
use serde_json;

#[tauri::command]
pub(crate) async fn validate_project_path(
    project_path: String,
) -> Result<project::PathValidationResult, String> {
    Ok(crate::check_project_path(&project_path))
}

/// 获取项目文件列表
///
/// 使用 walkdir 递归遍历项目目录（最大深度 5），跳过 .git、node_modules、
/// target 等构建产物目录，同时跳过隐藏文件（以 . 开头），但保留 .env.example。
#[tauri::command]
pub(crate) async fn get_project_files(
    project_path: String,
) -> Result<Vec<project::FileEntry>, String> {
    let project = std::path::Path::new(&project_path);
    if !project.exists() || !project.is_dir() {
        return Ok(vec![]);
    }

    // 需要跳过的目录名
    const SKIP_DIRS: &[&str] = &[
        ".git",
        "node_modules",
        "target",
        "__pycache__",
        "dist",
        ".next",
        "build",
        "coverage",
    ];

    let mut entries: Vec<project::FileEntry> = Vec::new();

    for entry in walkdir::WalkDir::new(&project_path)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        // 跳过根目录自身
        if entry.path() == project {
            continue;
        }

        // 计算相对路径
        let rel_path = entry
            .path()
            .strip_prefix(&project_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();

        // 检查路径的每一级是否在排除目录中
        let is_skipped = rel_path
            .split('/')
            .any(|component| SKIP_DIRS.contains(&component));
        if is_skipped {
            continue;
        }

        // 跳过隐藏文件/目录（以 . 开头），但保留 .env.example 等 .env* 文件
        if let Some(file_name) = entry.file_name().to_str() {
            if file_name.starts_with('.') && !file_name.starts_with(".env") {
                continue;
            }
        }

        let is_dir = entry.file_type().is_dir();
        let file_type = if is_dir {
            String::new()
        } else {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };

        entries.push(project::FileEntry {
            path: rel_path,
            is_dir,
            file_type,
        });
    }

    Ok(entries)
}

/// 项目入口结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct ProjectEntryResult {
    pub project: project::Project,
    pub is_recovery: bool,
    pub message: String,
}

/// 从硬盘加载项目数据（Tauri 命令）
/// 项目不存在时返回明确错误，不再返回默认空项目。
#[tauri::command]
pub(crate) async fn get_project(project_name: String) -> Result<project::Project, String> {
    let name = if project_name.is_empty() {
        return Err("未指定项目名称".to_string());
    } else {
        project_name
    };
    crate::load_project(&name)
}

#[tauri::command]
pub(crate) async fn check_engine_health(
    execution_profile: project::ExecutionProfile,
) -> Result<crate::engine::EngineHealth, String> {
    crate::engine::validate_profile(&execution_profile)?;
    Ok(crate::engine::check_engine_health(&execution_profile).await)
}

#[tauri::command]
pub(crate) async fn verify_engine_authentication(
    execution_profile: project::ExecutionProfile,
) -> Result<crate::engine::EngineAuthenticationResult, String> {
    crate::engine::verify_engine_authentication(&execution_profile).await
}

async fn ensure_engine_available(
    execution_profile: &project::ExecutionProfile,
) -> Result<crate::engine::PreparedEngine, String> {
    let prepared = crate::engine::prepare_engine(execution_profile).await?;
    if prepared.health.status.blocks_execution() {
        Err(format!("执行引擎不可用：{}", prepared.health.message))
    } else {
        Ok(prepared)
    }
}

fn execution_profile_change_blocker(project: &project::Project) -> Option<&'static str> {
    let waiting_engine = project
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| recovery.phase == project::RecoveryPhase::WaitingEngine);
    if project
        .execution_session
        .as_ref()
        .is_some_and(|session| session.active)
        && !waiting_engine
    {
        return Some("存在活跃执行会话，不能切换执行引擎");
    }
    if project
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| {
            matches!(
                recovery.phase,
                project::RecoveryPhase::Diagnosing
                    | project::RecoveryPhase::Repairing
                    | project::RecoveryPhase::Retesting
                    | project::RecoveryPhase::Replanning
            )
        })
    {
        return Some("错误恢复正在进行，不能切换执行引擎");
    }
    if project
        .workflow_state
        .autopilot_state
        .as_ref()
        .is_some_and(|autopilot| {
            autopilot.active
                && autopilot.run_status == project::AutopilotRunStatus::Running
                && !waiting_engine
        })
    {
        return Some("自动驾驶正在推进，暂停后才能切换执行引擎");
    }
    if project
        .workflow_state
        .managed_flow_state
        .as_ref()
        .is_some_and(|managed| {
            managed.active && managed.run_status == project::ManagedRunStatus::Running
        })
    {
        return Some("托管流程正在推进，暂停后才能切换执行引擎");
    }
    None
}

fn apply_execution_profile(
    project: &mut project::Project,
    mut execution_profile: project::ExecutionProfile,
) -> bool {
    let unchanged = project.execution_profile.runtime == execution_profile.runtime
        && project.execution_profile.provider == execution_profile.provider
        && project.execution_profile.permission_profile == execution_profile.permission_profile;
    if unchanged {
        return false;
    }
    execution_profile.profile_revision =
        project.execution_profile.profile_revision.saturating_add(1);
    project.execution_profile = execution_profile;
    project.workflow_state.data_revision = project.workflow_state.data_revision.saturating_add(1);
    project.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    true
}

fn apply_entry_execution_profile(
    project: &mut project::Project,
    execution_profile: project::ExecutionProfile,
) -> Result<bool, String> {
    let changed = project.execution_profile.runtime != execution_profile.runtime
        || project.execution_profile.provider != execution_profile.provider
        || project.execution_profile.permission_profile != execution_profile.permission_profile;
    if !changed {
        return Ok(false);
    }
    if let Some(message) = execution_profile_change_blocker(project) {
        return Err(message.to_string());
    }
    Ok(apply_execution_profile(project, execution_profile))
}

#[tauri::command]
pub(crate) async fn update_execution_profile(
    state: tauri::State<'_, AppState>,
    project_name: String,
    expected_data_revision: u64,
    execution_profile: project::ExecutionProfile,
) -> Result<project::Project, String> {
    crate::engine::validate_profile(&execution_profile)?;
    let pipeline_guard = state.pipeline_state.lock().await;
    if pipeline_guard
        .as_ref()
        .is_some_and(|pipeline| pipeline.status == crate::pipeline::PipelineStatus::Running)
    {
        return Err("执行正在运行，不能切换执行引擎".to_string());
    }

    let mut project = crate::load_project(&project_name)?;
    if project.workflow_state.data_revision != expected_data_revision {
        return Err(format!(
            "项目状态已更新，请同步后重试（当前修订 {}，请求修订 {}）",
            project.workflow_state.data_revision, expected_data_revision
        ));
    }
    if let Some(message) = execution_profile_change_blocker(&project) {
        return Err(message.to_string());
    }
    let prepared_engine = ensure_engine_available(&execution_profile).await?;
    let old_profile = project.execution_profile.clone();
    let waiting_engine = project
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| recovery.phase == project::RecoveryPhase::WaitingEngine);
    if !apply_execution_profile(&mut project, execution_profile) {
        return Ok(project);
    }
    if waiting_engine {
        if let Some(session) = project.execution_session.as_mut() {
            session.engine_snapshot = project.execution_profile.clone();
            session.engine_settings_revision = prepared_engine.settings().revision;
            session.engine_source_revision =
                if project.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    prepared_engine
                        .health
                        .source_revision
                        .clone()
                        .unwrap_or_default()
                } else {
                    String::new()
                };
            session.engine_api_backend =
                if project.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    prepared_engine
                        .settings()
                        .built_in_grok_build
                        .api_backend
                        .as_str()
                        .to_string()
                } else {
                    String::new()
                };
            session.engine_model =
                if project.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    prepared_engine.settings().built_in_grok_build.model.clone()
                } else {
                    String::new()
                };
            session.endpoint_fingerprint =
                if project.execution_profile.runtime == project::ExecutionRuntime::BuiltIn {
                    crate::settings::endpoint_fingerprint(
                        &prepared_engine.settings().built_in_grok_build.api_base_url,
                    )
                } else {
                    String::new()
                };
            session.engine_executable_path = prepared_engine
                .health
                .executable_path
                .clone()
                .unwrap_or_default();
        }
    }
    let audit_message = format!(
        "执行引擎配置已切换：{:?}/{} -> {:?}/{}；应用设置修订 {}",
        old_profile.runtime,
        old_profile.provider.display_name(),
        project.execution_profile.runtime,
        project.execution_profile.provider.display_name(),
        prepared_engine.settings().revision,
    );
    crate::pipeline::write_execution_history(
        &mut project,
        "info",
        project::ExecutionEventType::EngineProfileChanged,
        audit_message,
        None,
        None,
        None,
    );
    crate::save_and_reload_project(&project)
}

/// 初始化项目入口（Before 页面调用）
/// 创建新项目或安全恢复同名同路径项目。
///
/// 恢复时区分三种情况：
/// - 有效恢复：同名同路径，无陈旧执行会话
/// - 陈旧会话清理后恢复：同名同路径，有已失效的 execution_session 需清理
/// - 拒绝恢复：同名不同路径（名称冲突）
#[tauri::command]
pub(crate) async fn initialize_project_entry(
    project_name: String,
    project_path: String,
    entry_kind: String,
    execution_profile: Option<project::ExecutionProfile>,
) -> Result<project::Project, String> {
    if project_name.trim().is_empty() {
        return Err("项目名称不能为空".to_string());
    }
    if project_path.trim().is_empty() {
        return Err("项目路径不能为空".to_string());
    }

    let kind = match entry_kind.as_str() {
        "NoProject" => project::ProjectEntryKind::NoProject,
        "HalfProject" => project::ProjectEntryKind::HalfProject,
        _ => return Err(format!("未知的项目来源类型：{}", entry_kind)),
    };
    let mut selected_profile = execution_profile.unwrap_or_default();
    let _prepared_engine = ensure_engine_available(&selected_profile).await?;

    let path = std::path::Path::new(&project_path);

    // === 先检查同名项目是否已存在（冲突检测在目录操作之前） ===
    let project_data_path = crate::project_data_path(&project_name)?;
    if project_data_path.exists() {
        match crate::load_project(&project_name) {
            Ok(mut existing) => {
                if existing.project_path == project_path && !existing.project_path.is_empty() {
                    // Same name + same path — check for stale execution session
                    let had_stale_session = clean_stale_execution_session(&mut existing);

                    // Check for stale autopilot state (autopilot active but target milestone gone)
                    let had_stale_autopilot = clean_stale_autopilot_state(&mut existing);

                    // 修复旧版本留下的大阶段检查/托管矛盾状态。
                    let managed_milestone_reconciled =
                        crate::commands::workflow::reconcile_managed_milestone_project(
                            &mut existing,
                        );

                    let profile_changed =
                        apply_entry_execution_profile(&mut existing, selected_profile.clone())?;

                    if managed_milestone_reconciled {
                        existing.workflow_state.data_revision += 1;
                        existing.workflow_state.last_transition_at =
                            chrono::Utc::now().to_rfc3339();
                    }

                    if had_stale_session
                        || had_stale_autopilot
                        || managed_milestone_reconciled
                        || profile_changed
                    {
                        // Persist the cleaned state and reload
                        existing = crate::save_and_reload_project(&existing)?;
                    }

                    // Verify project path still exists
                    let proj_path = std::path::Path::new(&existing.project_path);
                    if !proj_path.exists() || !proj_path.is_dir() {
                        // Project path no longer valid — reset to Before
                        existing.workflow_state.top_level_phase = project::TopLevelPhase::Before;
                        existing.workflow_state.current_step = project::WorkflowStep::WaitingEntry;
                        existing.workflow_state.data_revision += 1;
                        existing = crate::save_and_reload_project(&existing)?;
                    }

                    return Ok(existing);
                } else if !existing.project_path.is_empty() {
                    return Err(format!(
                        "项目名称「{}」已被使用（路径：{}），请修改项目名称",
                        project_name, existing.project_path
                    ));
                }
            }
            Err(e) => {
                // 项目数据文件存在但无法解析（损坏或空文件）
                // 删除损坏的数据文件，允许从头创建
                let _ = std::fs::remove_file(&project_data_path);
                eprintln!(
                    "项目数据文件损坏，已删除并重新创建：{}（{}）",
                    project_data_path.display(),
                    e
                );
            }
        }
    }

    // NoProject: validate path and optionally create directory
    if kind == project::ProjectEntryKind::NoProject {
        if path.exists() {
            // Path exists but is a regular file — reject
            if !path.is_dir() {
                return Err(format!(
                    "路径「{}」已存在但是一个普通文件，不是目录。请选择目录路径。",
                    project_path
                ));
            }
            // Path exists, is a directory — check if non-empty
            let is_empty = std::fs::read_dir(path)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(false);
            if !is_empty {
                return Err(format!(
                    "目录「{}」非空，无法作为 No Project 使用。请选择空目录或使用 Half Project 改造已有项目。",
                    project_path
                ));
            }
        } else {
            // Path doesn't exist — create it
            std::fs::create_dir_all(path).map_err(|e| format!("创建项目目录失败：{}", e))?;
            // Verify the created directory is writable
            if !path.is_dir() {
                return Err(format!(
                    "目录「{}」创建后不可用，请检查权限或选择其他路径。",
                    project_path
                ));
            }
        }
    }

    selected_profile.profile_revision = 1;
    let mut project = if kind == project::ProjectEntryKind::HalfProject {
        project::Project::new_half(&project_name, &project_path)
    } else {
        let mut p = project::Project::new(&project_name);
        p.project_path = project_path.to_string();
        p
    };
    project.execution_profile = selected_profile;

    // Set workflow state based on entry kind
    match kind {
        project::ProjectEntryKind::NoProject => {
            project.workflow_state.top_level_phase = project::TopLevelPhase::FirstDiscussion;
            project.workflow_state.current_step = project::WorkflowStep::Discussion;
        }
        project::ProjectEntryKind::HalfProject => {
            // HalfProject 初始阶段必须为 Before（尚未进入讨论）
            project.workflow_state.top_level_phase = project::TopLevelPhase::Before;
            project.workflow_state.current_step = project::WorkflowStep::ExistingAnalysis;
        }
    }

    // Persist
    crate::save_and_reload_project(&project)
}

/// 3.3 执行引擎流水线
/// 根据传入的中阶段信息和子任务列表，启动一个后台任务，逐个执行这些子任务，并实时更新执行状态（运行中、成功、失败等）
/// 启动后台流水线执行一组子任务，立即返回成功，执行进度保存在全局状态中，前端可查询。
/// 前端全量持久化（仅用于兼容旧路径，关键业务变更必须调用对应业务接口）
/// 返回值改为完整 Project，确保前端拿到磁盘最终事实。
/// 增加基础校验：名称和路径必须与磁盘一致，revision 不得低于磁盘值。
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn persist_project(project_json: String) -> Result<project::Project, String> {
    let incoming: project::Project =
        serde_json::from_str(&project_json).map_err(|e| format!("解析项目失败：{}", e))?;

    // 基础校验：名称非空
    if incoming.name.is_empty() {
        return Err("项目名称不能为空".to_string());
    }

    // 从磁盘加载当前事实进行校验
    let disk = crate::load_project(&incoming.name)?;

    // 路径不可变更
    if incoming.project_path != disk.project_path {
        return Err("不允许通过 persist_project 修改项目路径".to_string());
    }

    // 入口类型不可变更
    if incoming.entry_kind != disk.entry_kind {
        return Err("不允许通过 persist_project 修改项目入口类型".to_string());
    }

    // 修订防护：不允许用旧版本覆盖新版本
    if incoming.workflow_state.data_revision < disk.workflow_state.data_revision {
        return Err(format!(
            "修订冲突：传入修订 {} 低于磁盘修订 {}，拒绝覆盖",
            incoming.workflow_state.data_revision, disk.workflow_state.data_revision
        ));
    }

    // 使用 save_and_reload 确保前后端状态一致，返回磁盘最终事实
    crate::save_and_reload_project(&incoming)
}

/// 审批命令
/// 根据 project_id 和 mid_stage_id，找到对应的中阶段，把它的状态改为 "approved"，然后保存回文件
#[tauri::command]
pub(crate) async fn approve_mid_stage(
    project_id: String,
    mid_stage_id: String,
) -> Result<String, String> {
    // 使用统一加载方法
    let mut project = crate::load_project(&project_id)?;

    // 双层循环查找并批准 mid_stage
    let mut found = false;
    let mut current_milestone_index = 0;
    for (mi, milestone) in project.milestones.iter().enumerate() {
        for mid_stage in &milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                current_milestone_index = mi;
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return Err("未找到指定的中阶段".to_string());
    }
    // 将当前 mid_stage 标记为 Approved
    {
        let milestone = &mut project.milestones[current_milestone_index];
        for mid_stage in &mut milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                mid_stage.status = project::MidStageStatus::Approved;
                break;
            }
        }
    }
    // 查找下一个可推进的中阶段（在当前 milestone 内）
    let mut next_mid_stage_id: Option<String> = None;
    let mut next_milestone_id: Option<String> = None;
    let mut project_completed = false;

    {
        let milestone = &project.milestones[current_milestone_index];
        let mut found_current = false;
        for mid_stage in &milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                found_current = true;
                continue;
            }
            if found_current
                && (mid_stage.status == project::MidStageStatus::Pending
                    || mid_stage.status == project::MidStageStatus::Ready)
            {
                next_mid_stage_id = Some(mid_stage.id.clone());
                next_milestone_id = Some(milestone.id.clone());
                break;
            }
        }
    }

    // 如果当前 milestone 内没有下一个 mid_stage，标记当前 milestone 为 Completed
    if next_mid_stage_id.is_none() {
        project.milestones[current_milestone_index].status = project::MilestoneStatus::Completed;

        // 查找下一个 Pending/Ready 的大阶段
        for mi in (current_milestone_index + 1)..project.milestones.len() {
            let ms = &project.milestones[mi];
            if ms.status == project::MilestoneStatus::Pending
                || ms.status == project::MilestoneStatus::InProgress
            {
                if let Some(first_mid) = ms.mid_stages.first() {
                    next_mid_stage_id = Some(first_mid.id.clone());
                    next_milestone_id = Some(ms.id.clone());
                    break;
                }
            }
        }

        if next_mid_stage_id.is_none() {
            project.status = project::ProjectStatus::Completed;
            project_completed = true;
        }
    }

    // 将找到的下一中阶段设为 Ready
    if let Some(ref next_mid_id) = next_mid_stage_id {
        for milestone in &mut project.milestones {
            for mid_stage in &mut milestone.mid_stages {
                if mid_stage.id == *next_mid_id {
                    mid_stage.status = project::MidStageStatus::Ready;
                    break;
                }
            }
        }
    }

    // 使用统一的 save_and_reload 确保前后端状态一致
    crate::save_and_reload_project(&project)?;

    // 构造返回值
    let result = serde_json::json!({
        "next_milestone_id": next_milestone_id,
        "next_mid_stage_id": next_mid_stage_id,
        "project_completed": project_completed,
    });
    Ok(result.to_string())
}
/// 拒绝指定的中阶段：把它的状态改成 "rejected"，然后保存回项目文件
#[tauri::command]
pub(crate) async fn reject_mid_stage(
    project_id: String,
    mid_stage_id: String,
) -> Result<(), String> {
    let mut project = crate::load_project(&project_id)?;
    let mut found = false;
    for milestone in &mut project.milestones {
        for mid_stage in &mut milestone.mid_stages {
            if mid_stage.id == mid_stage_id {
                mid_stage.status = project::MidStageStatus::Rejected;
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    if !found {
        return Err("未找到指定的中阶段".to_string());
    }
    crate::save_and_reload_project(&project)?;
    Ok(())
}

// ===================================================================
// 恢复辅助函数
// ===================================================================

/// 清理陈旧的 execution_session。
///
/// 以下情况视为陈旧：
/// - session 状态为 "executing" 但对应小阶段状态不是 Executing（进程已死但磁盘状态未更新）
/// - session 状态为 "awaiting_confirmation" 但对应小阶段不存在或状态不对
/// - session 的 milestone_id / mid_stage_id 与当前 Project 不匹配
///
/// 返回 true 表示清理了陈旧会话。
fn clean_stale_execution_session(proj: &mut project::Project) -> bool {
    let session = match proj.execution_session.as_ref() {
        Some(s) => s.clone(),
        None => return false,
    };

    let mut should_clean = false;

    // Check 1: session is active but refers to non-existent entities
    if session.active {
        // Find the referenced subtask
        let subtask_exists = proj
            .milestones
            .iter()
            .filter(|ms| ms.id == session.milestone_id)
            .flat_map(|ms| ms.mid_stages.iter())
            .filter(|mid| mid.id == session.mid_stage_id)
            .flat_map(|mid| mid.subtasks.iter())
            .any(|st| st.id == session.subtask_id);

        if !subtask_exists {
            // Referenced subtask no longer exists — stale session
            should_clean = true;
        } else if session.status == "executing" {
            // Check if subtask is still marked as Executing
            let still_executing = proj
                .milestones
                .iter()
                .filter(|ms| ms.id == session.milestone_id)
                .flat_map(|ms| ms.mid_stages.iter())
                .filter(|mid| mid.id == session.mid_stage_id)
                .flat_map(|mid| mid.subtasks.iter())
                .any(|st| {
                    st.id == session.subtask_id && st.status == project::SubtaskStatus::Executing
                });

            if !still_executing {
                // Session says executing but subtask doesn't — process died
                should_clean = true;
            }
        }

        // Check 2: current milestone/mid_stage mismatch
        if !session.milestone_id.is_empty() && proj.current_milestone_id != session.milestone_id {
            should_clean = true;
        }
        if !session.mid_stage_id.is_empty() && proj.current_mid_stage_id != session.mid_stage_id {
            should_clean = true;
        }
    } else {
        // Inactive session is just stale data
        should_clean = true;
    }

    if should_clean {
        proj.execution_session = None;
        // If current step is Execution with stale session, go back to MidStageSelection
        if proj.workflow_state.current_step == project::WorkflowStep::Execution {
            proj.workflow_state.current_step = project::WorkflowStep::MidStageSelection;
            proj.workflow_state.data_revision += 1;
            proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        }
    }

    should_clean
}

/// 清理陈旧的 autopilot 状态。
///
/// 以下情况视为陈旧：
/// - autopilot_active 为 true 但 autopilot_state 为 None
/// - autopilot_target_milestone_id 指向不存在的大阶段
/// - autopilot 激活但所有大阶段已完成
///
/// 返回 true 表示清理了。
fn clean_stale_autopilot_state(proj: &mut project::Project) -> bool {
    if !proj.workflow_state.autopilot_active {
        // Deactivate if active flag is false but state still exists
        if proj.workflow_state.autopilot_state.is_some() {
            proj.workflow_state.autopilot_state = None;
            proj.workflow_state.autopilot_target_milestone_id = String::new();
            return true;
        }
        return false;
    }

    let mut should_clean = false;

    // Check: autopilot state missing
    if proj.workflow_state.autopilot_state.is_none() {
        should_clean = true;
    }

    // Check: target milestone exists
    let target_id = &proj.workflow_state.autopilot_target_milestone_id;
    if !target_id.is_empty() {
        let target_exists = proj.milestones.iter().any(|m| m.id == *target_id);
        if !target_exists {
            should_clean = true;
        }
    }

    // Check: all milestones completed
    let all_completed = !proj.milestones.is_empty()
        && proj
            .milestones
            .iter()
            .all(|m| m.status == project::MilestoneStatus::Completed);
    if all_completed {
        should_clean = true;
    }

    if should_clean {
        proj.workflow_state.autopilot_active = false;
        proj.workflow_state.autopilot_target_milestone_id = String::new();
        proj.workflow_state.autopilot_state = None;
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    }

    should_clean
}

#[cfg(test)]
mod execution_profile_tests {
    use super::*;

    fn codex_profile() -> project::ExecutionProfile {
        project::ExecutionProfile {
            runtime: project::ExecutionRuntime::Plugin,
            provider: project::ExecutionProvider::Codex,
            permission_profile: project::PermissionProfile::Unattended,
            profile_revision: 1,
        }
    }

    #[test]
    fn profile_update_increments_both_revisions() {
        let mut project = project::Project::new("profile-update");
        project.workflow_state.data_revision = 7;
        assert!(apply_execution_profile(&mut project, codex_profile()));
        assert_eq!(
            project.execution_profile.provider,
            project::ExecutionProvider::Codex
        );
        assert_eq!(project.execution_profile.profile_revision, 2);
        assert_eq!(project.workflow_state.data_revision, 8);
        assert!(!apply_execution_profile(&mut project, codex_profile()));
        assert_eq!(project.workflow_state.data_revision, 8);
    }

    #[test]
    fn active_session_and_recovery_block_changes() {
        let mut project = project::Project::new("profile-blocked");
        project.execution_session = Some(project::ExecutionSession {
            active: true,
            ..Default::default()
        });
        assert!(execution_profile_change_blocker(&project)
            .unwrap()
            .contains("活跃执行会话"));

        project.execution_session = None;
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            phase: project::RecoveryPhase::Repairing,
            ..Default::default()
        });
        assert!(execution_profile_change_blocker(&project)
            .unwrap()
            .contains("错误恢复"));
    }

    #[test]
    fn waiting_engine_allows_profile_change() {
        let mut project = project::Project::new("profile-engine-blocked");
        project.execution_session = Some(project::ExecutionSession {
            active: true,
            ..Default::default()
        });
        project.workflow_state.recovery_state = Some(project::RecoveryState {
            phase: project::RecoveryPhase::WaitingEngine,
            error_kind: project::RecoveryErrorKind::EngineBlocked,
            ..Default::default()
        });
        assert!(execution_profile_change_blocker(&project).is_none());
    }

    #[test]
    fn entry_profile_is_applied_at_a_stable_boundary_and_never_silently_ignored() {
        let mut project = project::Project::new("entry-profile");
        assert!(apply_entry_execution_profile(&mut project, codex_profile()).unwrap());
        assert_eq!(
            project.execution_profile.provider,
            project::ExecutionProvider::Codex
        );

        project.execution_session = Some(project::ExecutionSession {
            active: true,
            ..Default::default()
        });
        assert!(
            apply_entry_execution_profile(&mut project, project::ExecutionProfile::default())
                .is_err()
        );
    }
}
