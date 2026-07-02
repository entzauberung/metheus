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

/// 从硬盘加载项目数据（Tauri 命令）
#[tauri::command]
pub(crate) async fn get_project(project_name: String) -> Result<project::Project, String> {
    let name = if project_name.is_empty() {
        "我的游戏".to_string()
    } else {
        project_name
    };
    match crate::load_project(&name) {
        Ok(project) => Ok(project),
        Err(_) => {
            // 文件不存在时返回默认空项目，不报错
            Ok(project::Project::new(&name))
        }
    }
}

/// 3.3 执行引擎流水线
/// 根据传入的中阶段信息和子任务列表，启动一个后台任务，逐个执行这些子任务，并实时更新执行状态（运行中、成功、失败等）
/// 启动后台流水线执行一组子任务，立即返回成功，执行进度保存在全局状态中，前端可查询。
#[tauri::command]
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
