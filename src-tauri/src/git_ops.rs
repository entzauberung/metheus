use crate::project;
use dirs;
use serde_json;

/// 在项目目录下执行 git add . → git commit --allow-empty → git tag -f
/// 专业模式：从中阶段完成处调用（version = "v0.1.1"）
/// 快速模式：从大阶段完成处调用（version = "v0.1"）
///
/// 返回 tag 名，如 "metheus/v0.1.1"
#[tauri::command]
pub(crate) async fn git_save_node(
    project_path: String,
    version: String,
    title: String,
) -> Result<String, String> {
    // 1. git add . 加暂存
    let add_output = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git add失败: {}", e))?;
    if !add_output.status.success() {
        return Err(format!(
            "git add 执行失败：\n{}",
            String::from_utf8_lossy(&add_output.stderr)
        ));
    }
    // 2. git commit -m 记录文档
    // --allow-empty 确保即使没有文件变更也能提交
    //    （比如一个中阶段只改了文案，没有代码变更）
    let commit_message = format!("【弥】节点 {}: {}", version, title);
    let commit_output = std::process::Command::new("git")
        .args(["commit", "-m", &commit_message, "--allow-empty"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git commit 执行失败：{}", e))?;
    if !commit_output.status.success() {
        // "nothing to commit"不是错误，只是没有变更
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if !stderr.contains("nothing to commit") {
            return Err(format!("git commit 执行失败:\n{}", stderr));
        }
    }
    // 3. git tag 打标签
    // tag 格式 metheus/v0.1.1，用 metheus/ 前缀避免和用户自己的 tag 冲突
    // -f 允许覆盖已有 tag（如果同一个节点重做后重新存档）
    let tag_name = format!("metheus/{}", version);
    let tag_output = std::process::Command::new("git")
        .args(["tag", "-f", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git tag 失败: {}", e))?;
    if !tag_output.status.success() {
        return Err(format!(
            "git tag 执行失败: \n{}",
            String::from_utf8_lossy(&tag_output.stderr)
        ));
    }
    // 返回 tag 名， 让调用方决定写回哪个节点
    Ok(tag_name)
}

/// 小阶段 Git 存档命令
///
/// 在项目目录下执行 git add . → git commit --allow-empty → git tag -f
/// tag 格式：metheus/auto/{mid_stage_version}/task-{subtask_index}
/// 返回 tag 名，调用方可写回 Subtask.auto_tag
#[tauri::command]
pub(crate) async fn git_save_subtask(
    project_path: String,
    subtask_index: u32,
    mid_stage_version: String,
    subtask_title: String,
) -> Result<String, String> {
    // 1. git add . 暂存所有变更
    let add_output = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git add 失败: {}", e))?;
    if !add_output.status.success() {
        return Err(format!(
            "git add 执行失败：\n{}",
            String::from_utf8_lossy(&add_output.stderr)
        ));
    }

    // 2. git commit（--allow-empty 确保即使无文件变更也能提交）
    let commit_message = format!(
        "【弥】小阶段 {}/{}：{}",
        subtask_index, mid_stage_version, subtask_title
    );
    let commit_output = std::process::Command::new("git")
        .args(["commit", "-m", &commit_message, "--allow-empty"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git commit 执行失败：{}", e))?;
    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if !stderr.contains("nothing to commit") {
            return Err(format!("git commit 执行失败:\n{}", stderr));
        }
    }

    // 3. git tag -f（覆盖已有 tag）
    let tag_name = format!("metheus/auto/{}/task-{}", mid_stage_version, subtask_index);
    let tag_output = std::process::Command::new("git")
        .args(["tag", "-f", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git tag 失败: {}", e))?;
    if !tag_output.status.success() {
        return Err(format!(
            "git tag 执行失败:\n{}",
            String::from_utf8_lossy(&tag_output.stderr)
        ));
    }

    Ok(tag_name)
}

/// Git 回退命令
///
/// 把项目代码和执行树状态一起回退到指定 tag 对应的版本
/// 1. 检查工作区是否有未提交变更 → 有则 stash
/// 2. git reset --hard 到目标 tag → 代码回退
/// 3. 遍历 project.json → 回退点之后的节点标记为 RolledBack
#[tauri::command]
pub(crate) async fn git_rollback_to_mid_stage(
    project_path: String,
    tag_name: String,
    project_id: String,
) -> Result<String, String> {
    // 1. 检查工作区是否有未提交变更
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git status 失败: {}", e))?;
    let status = String::from_utf8_lossy(&status_output.stdout);
    let has_uncommitted = !status.trim().is_empty();
    // 如有未提交变更，先 stash 起来，避免被 reset --hard 永久清除
    if has_uncommitted {
        let stash_output = std::process::Command::new("git")
            .args(["stash", "push", "-m", "metheus_rollback_auto_stash"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git stash 失败: {}", e))?;
        if !stash_output.status.success() {
            return Err(format!(
                "git stash 执行失败:\n{}",
                String::from_utf8_lossy(&stash_output.stderr)
            ));
        }
    }
    // 2. git reset --hard 到目标 tag
    let reset_output = std::process::Command::new("git")
        .args(["reset", "--hard", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git reset 失败: {}", e))?;
    if !reset_output.status.success() {
        return Err(format!(
            "回退到 {} 失败:\n{}",
            tag_name,
            String::from_utf8_lossy(&reset_output.stderr)
        ));
    }
    // 2.5 清理被跳过节点的 Git tag
    // 遍历所有 mid_stage，删除版本号大于目标版本的节点的 git tag
    {
        let target_version = tag_name
            .strip_prefix("metheus/")
            .unwrap_or(&tag_name)
            .to_string();
        let pp = std::path::Path::new(&project_path);
        let md = pp.join(".metheus");
        let pf = md.join(format!("{}.json", project_id));
        if pf.exists() {
            if let Ok(content) = std::fs::read_to_string(&pf) {
                if let Ok(proj) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(milestones) = proj["milestones"].as_array() {
                        for milestone in milestones {
                            if let Some(mid_stages) = milestone["mid_stages"].as_array() {
                                for mid in mid_stages {
                                    let version = mid["version"].as_str().unwrap_or("");
                                    let git_tag = mid["git_tag"].as_str().unwrap_or("");
                                    if !git_tag.is_empty()
                                        && compare_version_strings(version, &target_version) > 0
                                    {
                                        match std::process::Command::new("git")
                                            .args(["tag", "-d", git_tag])
                                            .current_dir(&project_path)
                                            .output()
                                        {
                                            Ok(output) => {
                                                if !output.status.success() {
                                                    eprintln!(
                                                        "警告: 删除 git tag {} 失败: {}",
                                                        git_tag,
                                                        String::from_utf8_lossy(&output.stderr)
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "警告: 执行 git tag -d {} 失败: {}",
                                                    git_tag, e
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // 3. 更新 project.json 中的执行树状态
    // 从tag_name 中提取版本号，去掉 "metheus/" 前缀
    let target_version = tag_name
        .strip_prefix("metheus/")
        .unwrap_or(&tag_name)
        .to_string();
    // 读取 project.json
    let project_path_obj = std::path::Path::new(&project_path);
    let metheus_dir = project_path_obj.join(".metheus");
    let project_file = metheus_dir.join(format!("{}.json", project_id));
    if !project_file.exists() {
        return Err(format!("项目文件不存在: {}", project_file.display()));
    }
    let content =
        std::fs::read_to_string(&project_file).map_err(|e| format!("读取项目文件失败: {}", e))?;
    let mut project: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败: {}", e))?;
    // 遍历 milestones -> mid_stages, 标记回点之后的节点
    if let Some(milestones) = project["milestones"].as_array_mut() {
        for milestone in milestones.iter_mut() {
            if let Some(mid_stages) = milestone["mid_stages"].as_array_mut() {
                for mid in mid_stages.iter_mut() {
                    let version = mid["version"].as_str().unwrap_or("");
                    // 比较版本号：如果当前节点版本 > 目标版本，标记为 RolledBack
                    if compare_version_strings(version, &target_version) > 0 {
                        mid["status"] = serde_json::Value::String("RolledBack".to_string());
                    }
                }
            }
        }
    }
    // 写回文件
    let json_str =
        serde_json::to_string_pretty(&project).map_err(|e| format!("序列化项目文件失败: {}", e))?;
    std::fs::write(&project_file, &json_str).map_err(|e| format!("写入项目文件失败: {}", e))?;
    // 组装返回消息
    let stash_note = if has_uncommitted {
        "\n（你有未提交的变更已被临时存储，回退完成后可执行 git stash pop 恢复）"
    } else {
        ""
    };
    Ok(format!("已回退到 {}{}", tag_name, stash_note))
}

/// Git 回退到指定小阶段
///
/// 把项目代码回退到指定 subtask auto_tag 对应的版本。
/// 与 git_rollback_to_mid_stage 的区别：回退粒度更细，只回退到某个小阶段（而非中阶段）。
/// 1. 检查工作区是否有未提交变更 → 有则 stash
/// 2. git reset --hard 到目标 tag → 代码回退
/// 3. 遍历 project.json → 回退点之后的 subtasks 和 mid_stages 标记为 RolledBack
#[tauri::command]
pub(crate) async fn git_rollback_to_subtask(
    project_path: String,
    project_id: String,
    tag_name: String,
) -> Result<String, String> {
    // 1. 检查工作区是否有未提交变更
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git status 失败: {}", e))?;
    let status = String::from_utf8_lossy(&status_output.stdout);
    let has_uncommitted = !status.trim().is_empty();
    // 如有未提交变更，先 stash 起来
    if has_uncommitted {
        let stash_output = std::process::Command::new("git")
            .args(["stash", "push", "-m", "metheus_rollback_auto_stash"])
            .current_dir(&project_path)
            .output()
            .map_err(|e| format!("git stash 失败: {}", e))?;
        if !stash_output.status.success() {
            return Err(format!(
                "git stash 执行失败:\n{}",
                String::from_utf8_lossy(&stash_output.stderr)
            ));
        }
    }

    // 2. git reset --hard 到目标 tag
    let reset_output = std::process::Command::new("git")
        .args(["reset", "--hard", &tag_name])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git reset 失败: {}", e))?;
    if !reset_output.status.success() {
        // reset 失败，尝试恢复 stash
        if has_uncommitted {
            let _ = std::process::Command::new("git")
                .args(["stash", "pop"])
                .current_dir(&project_path)
                .output();
        }
        return Err(format!(
            "回退到 {} 失败:\n{}",
            tag_name,
            String::from_utf8_lossy(&reset_output.stderr)
        ));
    }

    // 3. 更新 project.json 中的执行树状态
    // 解析 tag_name 获取目标信息：格式 metheus/auto/{version}/task-{index}
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let project_file = std::path::Path::new(&home)
        .join(".metheus")
        .join(format!("{}.json", project_id));
    if !project_file.exists() {
        return Err(format!("项目文件不存在: {}", project_file.display()));
    }
    let content =
        std::fs::read_to_string(&project_file).map_err(|e| format!("读取项目文件失败: {}", e))?;
    let mut project: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析项目文件失败: {}", e))?;

    // 找到目标 subtask 所属的 mid_stage，标记其后的 subtasks 和 mid_stages 为 RolledBack
    let mut target_found = false;
    let mut target_mid_stage_id = String::new();
    let mut passed_target = false;

    if let Some(milestones) = project["milestones"].as_array_mut() {
        for milestone in milestones.iter_mut() {
            if let Some(mid_stages) = milestone["mid_stages"].as_array_mut() {
                for mid in mid_stages.iter_mut() {
                    // 提前提取 mid_id，避免后续 borrow checker 冲突
                    let mid_id = mid["id"].as_str().unwrap_or("").to_string();

                    if passed_target {
                        // 目标之后的 mid_stage → 标记为 RolledBack
                        mid["status"] = serde_json::Value::String("RolledBack".to_string());
                        continue;
                    }

                    if let Some(subtasks) = mid["subtasks"].as_array_mut() {
                        for subtask in subtasks.iter_mut() {
                            let auto_tag = subtask["auto_tag"].as_str().unwrap_or("");
                            if auto_tag == tag_name {
                                target_found = true;
                                target_mid_stage_id = mid_id.clone();
                                // 当前 subtask 保持原状态
                            } else if target_found && mid_id == target_mid_stage_id {
                                // 同一 mid_stage 内，目标 subtask 之后的 subtask
                                subtask["status"] =
                                    serde_json::Value::String("RolledBack".to_string());
                            }
                        }
                    }

                    if target_found && mid_id == target_mid_stage_id {
                        // 当前 mid_stage 完成后，后续 mid_stages 标记为 RolledBack
                        passed_target = true;
                    }
                }
            }
        }
    }

    if !target_found {
        return Err(format!("未找到 tag {} 对应的小阶段", tag_name));
    }

    // 写回文件
    let json_str =
        serde_json::to_string_pretty(&project).map_err(|e| format!("序列化项目文件失败: {}", e))?;
    std::fs::write(&project_file, &json_str).map_err(|e| format!("写入项目文件失败: {}", e))?;

    // 组装返回消息
    let stash_note = if has_uncommitted {
        "\n（你有未提交的变更已被临时存储，回退完成后可执行 git stash pop 恢复）"
    } else {
        ""
    };
    Ok(format!("已回退到 {}{}", tag_name, stash_note))
}

/// 获取 metheus/ 前缀的 Git tag 摘要列表
///
/// 执行 git tag -l "metheus/*" --sort=-creatordate 获取所有 metheus tag，
/// 解析为 GitTagInfo 列表返回。非 git 仓库或无匹配 tag 时返回空数组。
#[tauri::command]
pub(crate) async fn get_git_tags_summary(
    project_path: String,
) -> Result<Vec<project::GitTagInfo>, String> {
    let output = std::process::Command::new("git")
        .args([
            "tag",
            "-l",
            "metheus/*",
            "--sort=-creatordate",
            "--format=%(refname:short)|%(creatordate:short)|%(subject)",
        ])
        .current_dir(&project_path)
        .output()
        .map_err(|e| format!("git tag 命令执行失败: {}", e))?;

    if !output.status.success() {
        // 非 git 仓库或无权限等情况，返回空数组
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tags: Vec<project::GitTagInfo> = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 3 {
            // 脏数据保护：跳过不满足 3 段的行
            continue;
        }
        tags.push(project::GitTagInfo {
            name: parts[0].to_string(),
            date: parts[1].to_string(),
            subject: parts[2].to_string(),
        });
    }

    Ok(tags)
}

/// 获取当前工作区的 git diff
///
/// 执行 git diff 获取未暂存的变更。非 git 仓库或工作区干净时返回空字符串。
#[tauri::command]
pub(crate) async fn get_current_diff(project_path: String) -> Result<String, String> {
    // 1. 检查 .git 是否存在（目录或文件，兼容 worktree）
    let git_path = std::path::Path::new(&project_path).join(".git");
    if !git_path.exists() {
        eprintln!("[get_current_diff] 不是 git 仓库，返回空");
        return Ok(String::new());
    }

    // 2. 执行 git diff
    let output = std::process::Command::new("git")
        .args(["diff"])
        .current_dir(&project_path)
        .output()
        .map_err(|e| {
            eprintln!("[get_current_diff] git 命令不可用: {}", e);
            format!("git 命令不可用: {}", e)
        })?;

    // 3. 退出码为零 → 返回 diff 内容（可能为空字符串 = 无变更）
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    // 4. 退出码非零 → 分析 stderr 区分场景
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    let stderr_lower = stderr_str.to_lowercase();

    if stderr_lower.contains("not a git repository") {
        eprintln!("[get_current_diff] 不是 git 仓库");
        return Ok(String::new());
    }

    if stderr_lower.contains("does not have any commits")
        || stderr_lower.contains("ambiguous argument")
        || stderr_lower.contains("unknown revision")
    {
        eprintln!("[get_current_diff] 仓库尚无提交");
        return Ok(String::new());
    }

    // 5. 未知错误 → 截断日志 + 返回空
    let truncated: String = stderr_str.chars().take(200).collect();
    eprintln!("[get_current_diff] 未知 git 错误: {}", truncated);
    Ok(String::new())
}

/// 比较两个版本号字符串（eg: "v0.1.1"  "v0.1.3"）
/// 返回 -1：a < b，0：a == b，1：a > b
pub(crate) fn compare_version_strings(a: &str, b: &str) -> i32 {
    let parts_a: Vec<u32> = a
        .strip_prefix('v')
        .unwrap_or(a)
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    let parts_b: Vec<u32> = b
        .strip_prefix('v')
        .unwrap_or(b)
        .split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    for i in 0..parts_a.len().max(parts_b.len()) {
        let num_a = parts_a.get(i).copied().unwrap_or(0);
        let num_b = parts_b.get(i).copied().unwrap_or(0);
        if num_a < num_b {
            return -1;
        }
        if num_a > num_b {
            return 1;
        }
    }
    0
}

/// 把 Git tag 名写入指定中阶段节点，并持久化到 project.json（辅助函数）
pub(crate) fn save_tag_to_mid_stage(
    project_id: &str,
    mid_stage_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    // 读取 project 文件, ~/.metheus/{project_id}.json
    let app_dir = dirs::home_dir().ok_or("无法获取 home 目录".to_string())?;
    let project_file = app_dir
        .join(".metheus")
        .join(format!("{}.json", project_id));

    let content = std::fs::read_to_string(&project_file)
        .map_err(|e| format!("读取 project 文件失败: {}", e))?;

    let mut project: project::Project =
        serde_json::from_str(&content).map_err(|e| format!("解析 project 文件失败: {}", e))?;

    // 遍历找到对应 mid_stage
    let mut found = false;
    for milestone in &mut project.milestones {
        for mid in &mut milestone.mid_stages {
            if mid.id == mid_stage_id {
                mid.git_tag = tag_name.to_string();
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }

    if !found {
        return Err(format!("未找到中阶段: {}", mid_stage_id));
    }

    // 写回文件
    let json = serde_json::to_string_pretty(&project)
        .map_err(|e| format!("序列化 project 失败: {}", e))?;

    std::fs::write(&project_file, json).map_err(|e| format!("写入 project 文件失败: {}", e))?;

    Ok(())
}
