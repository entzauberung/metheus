use crate::project;
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
pub(crate) async fn get_project_files(project_path: String) -> Result<Vec<project::FileEntry>, String> {
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

/// 初始化项目入口（Before 页面调用）
/// 创建新项目或安全恢复同名同路径项目。
#[tauri::command]
pub(crate) async fn initialize_project_entry(
    project_name: String,
    project_path: String,
    entry_kind: String,
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

    let path = std::path::Path::new(&project_path);

    // === 先检查同名项目是否已存在（冲突检测在目录操作之前） ===
    let project_data_path = crate::project_data_path(&project_name)?;
    if project_data_path.exists() {
        if let Ok(existing) = crate::load_project(&project_name) {
            if existing.project_path == project_path && !existing.project_path.is_empty() {
                // Same name + same path = recovery — don't recreate or reset
                return Ok(existing);
            } else if !existing.project_path.is_empty() {
                return Err(format!(
                    "项目名称「{}」已被使用（路径：{}），请修改项目名称",
                    project_name, existing.project_path
                ));
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
            std::fs::create_dir_all(path)
                .map_err(|e| format!("创建项目目录失败：{}", e))?;
            // Verify the created directory is writable
            if !path.is_dir() {
                return Err(format!(
                    "目录「{}」创建后不可用，请检查权限或选择其他路径。",
                    project_path
                ));
            }
        }
    }

    let mut project = if kind == project::ProjectEntryKind::HalfProject {
        project::Project::new_half(&project_name, &project_path)
    } else {
        let mut p = project::Project::new(&project_name);
        p.project_path = project_path.to_string();
        p
    };

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
    crate::save_project(&project)?;

    Ok(project)
}

/// 3.3 执行引擎流水线
/// 根据传入的中阶段信息和子任务列表，启动一个后台任务，逐个执行这些子任务，并实时更新执行状态（运行中、成功、失败等）
/// 启动后台流水线执行一组子任务，立即返回成功，执行进度保存在全局状态中，前端可查询。
#[tauri::command]
#[allow(dead_code)]
pub(crate) async fn persist_project(project_json: String) -> Result<String, String> {
    //前端发来的 JSON 字符串转成 Project 对象
    let project: project::Project =
        serde_json::from_str(&project_json).map_err(|e| format!("解析项目失败：{}", e))?;
    //调用已有的保存函数，把项目写入文件
    crate::save_project(&project)?;
    Ok("保存成功".to_string())
}

/// 审批命令
/// 根据 project_id 和 mid_stage_id，找到对应的中阶段，把它的状态改为 "approved"，然后保存回文件
#[tauri::command]
pub(crate) async fn approve_mid_stage(project_id: String, mid_stage_id: String) -> Result<String, String> {
    // 1. 获取home 目录
    let app_dir = dirs::home_dir().ok_or("无法获取 home 目录".to_string())?;
    // 2. 构造项目文件路径
    let project_file = app_dir
        .join(".metheus")
        .join(format!("{}.json", project_id));
    // 3. 读取文件内容
    let content =
        std::fs::read_to_string(&project_file).map_err(|e| format!("读取项目文件失败：{}", e))?;
    // 4. 解析为 Project 结构
    let mut project: project::Project =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败：{}", e))?;
    // 5. 双层循环查找并批准 mid_stage
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
    // 6. 查找下一个可推进的中阶段（在当前 milestone 内）
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
                // 找到下一个大阶段，将其第一个 mid_stage 设为 Ready
                if let Some(first_mid) = ms.mid_stages.first() {
                    next_mid_stage_id = Some(first_mid.id.clone());
                    next_milestone_id = Some(ms.id.clone());
                    break;
                }
            }
        }

        // 如果仍然没有找到下一个，标记项目完成
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

    // 序列化回 JSON 并写回文件
    let json = serde_json::to_string_pretty(&project).map_err(|e| format!("序列化失败：{}", e))?;
    std::fs::write(&project_file, json).map_err(|e| format!("保存失败：{}", e))?;

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
pub(crate) async fn reject_mid_stage(project_id: String, mid_stage_id: String) -> Result<(), String> {
    let app_dir = dirs::home_dir().ok_or("无法获取 home 目录".to_string())?;
    let project_path = app_dir
        .join(".metheus")
        .join(format!("{}.json", project_id));
    let content =
        std::fs::read_to_string(&project_path).map_err(|e| format!("读取项目文件失败: {}", e))?;
    let mut project: project::Project =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败: {}", e))?;
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
    let json = serde_json::to_string_pretty(&project).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&project_path, json).map_err(|e| format!("保存失败: {}", e))?;
    Ok(())
}
