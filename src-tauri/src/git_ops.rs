use crate::project;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_git(project_path: &str, args: &[&str], context: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("{}：{}", context, error))?;
    if !output.status.success() {
        return Err(format!(
            "{}：{}",
            context,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn run_git_with_index(
    project_path: &str,
    index_path: &Path,
    args: &[&str],
    context: &str,
) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_INDEX_FILE", index_path)
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("{}：{}", context, error))?;
    if !output.status.success() {
        return Err(format!(
            "{}：{}",
            context,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn head(project_path: &str) -> Result<String, String> {
    run_git(project_path, &["rev-parse", "HEAD"], "读取 Git HEAD 失败")
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
}

fn status_paths(project_path: &str) -> Result<Vec<String>, String> {
    let output = run_git(
        project_path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        "读取 Git 工作区状态失败",
    )?;
    let entries: Vec<&[u8]> = output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        if entry.len() >= 3 {
            let index_status = entry[0] as char;
            let worktree_status = entry[1] as char;
            paths.push(String::from_utf8_lossy(&entry[3..]).to_string());
            index += 1;
            if matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C') {
                if let Some(source) = entries.get(index) {
                    paths.push(String::from_utf8_lossy(source).to_string());
                }
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    Ok(paths)
}

fn ensure_clean_workspace(project_path: &str) -> Result<(), String> {
    let paths = status_paths(project_path)?;
    if paths.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "工作区存在未提交或未跟踪修改，拒绝执行 Git 操作：{}",
            paths.join("、")
        ))
    }
}

fn ensure_only_authorized_changes(
    project_path: &str,
    authorized_paths: &[String],
) -> Result<(), String> {
    let authorized: BTreeSet<&str> = authorized_paths.iter().map(String::as_str).collect();
    let outside: Vec<String> = status_paths(project_path)?
        .into_iter()
        .filter(|path| !authorized.contains(path.as_str()))
        .collect();
    if outside.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "工作区包含计划范围外变更，拒绝提交：{}",
            outside.join("、")
        ))
    }
}

/// 使用隔离的临时 index 捕获授权文件的完整 diff，不改变用户工作区的暂存状态。
pub(crate) fn capture_authorized_diff(
    project_path: &str,
    authorized_paths: &[String],
) -> Result<String, String> {
    if authorized_paths.is_empty() {
        return Err("小阶段授权文件范围为空，无法捕获变更".to_string());
    }
    ensure_only_authorized_changes(project_path, authorized_paths)?;

    let index_path =
        std::env::temp_dir().join(format!("metheus-git-index-{}", uuid::Uuid::new_v4()));
    let lock_path = PathBuf::from(format!("{}.lock", index_path.to_string_lossy()));
    let result = (|| {
        run_git_with_index(
            project_path,
            &index_path,
            &["read-tree", "HEAD"],
            "初始化临时 Git 索引失败",
        )?;

        let pathspecs: Vec<String> = authorized_paths
            .iter()
            .map(|path| format!(":(literal){}", path))
            .collect();
        let mut add_args = vec!["add", "-A", "--"];
        add_args.extend(pathspecs.iter().map(String::as_str));
        run_git_with_index(
            project_path,
            &index_path,
            &add_args,
            "在临时索引中暂存授权文件失败",
        )?;

        let mut diff_args = vec![
            "diff",
            "--cached",
            "--binary",
            "--no-ext-diff",
            "HEAD",
            "--",
        ];
        diff_args.extend(pathspecs.iter().map(String::as_str));
        run_git_with_index(
            project_path,
            &index_path,
            &diff_args,
            "读取小阶段授权变更失败",
        )
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
    })();

    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_file(lock_path);
    result
}

pub(crate) struct GeneratedFileUpdate {
    relative_path: String,
    original_content: String,
    updated_content: String,
}

impl GeneratedFileUpdate {
    pub(crate) fn constitution(original_content: String, updated_content: String) -> Self {
        Self {
            relative_path: "CONSTITUTION.md".to_string(),
            original_content,
            updated_content,
        }
    }

    fn changed(&self) -> bool {
        self.original_content != self.updated_content
    }
}

fn atomic_write_text(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("文件缺少父目录：{}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("文件名不是有效 UTF-8：{}", path.display()))?;
    let temp_path = parent.join(format!(
        ".{}.metheus-{}.tmp",
        file_name,
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, content)
        .map_err(|error| format!("写入临时文件 {} 失败：{}", temp_path.display(), error))?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temp_path, metadata.permissions()).map_err(|error| {
            let _ = fs::remove_file(&temp_path);
            format!("保留文件权限 {} 失败：{}", path.display(), error)
        })?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(&temp_path, path).map_err(|error| {
            let _ = fs::remove_file(&temp_path);
            format!("原子替换文件 {} 失败：{}", path.display(), error)
        })
    }

    #[cfg(windows)]
    {
        let backup_path = parent.join(format!(
            ".{}.metheus-{}.bak",
            file_name,
            uuid::Uuid::new_v4()
        ));
        fs::rename(path, &backup_path).map_err(|error| {
            let _ = fs::remove_file(&temp_path);
            format!("备份文件 {} 失败：{}", path.display(), error)
        })?;
        match fs::rename(&temp_path, path) {
            Ok(()) => {
                let _ = fs::remove_file(backup_path);
                Ok(())
            }
            Err(error) => {
                let restore_result = fs::rename(&backup_path, path);
                let _ = fs::remove_file(&temp_path);
                match restore_result {
                    Ok(()) => Err(format!("原子替换文件 {} 失败：{}", path.display(), error)),
                    Err(restore_error) => Err(format!(
                        "替换文件 {} 失败：{}；恢复备份也失败：{}",
                        path.display(),
                        error,
                        restore_error
                    )),
                }
            }
        }
    }
}

fn tag_target(project_path: &str, tag_name: &str) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            &format!("refs/tags/{}^{{}}", tag_name),
        ])
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("检查 Git 标签失败：{}", error))?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    } else {
        Ok(None)
    }
}

fn ensure_tag_available(project_path: &str, tag_name: &str) -> Result<bool, String> {
    let Some(existing) = tag_target(project_path, tag_name)? else {
        return Ok(false);
    };
    let current = head(project_path)?;
    if existing == current {
        ensure_clean_workspace(project_path)?;
        Ok(true)
    } else {
        Err(format!(
            "Git 标签 {} 已指向提交 {}，禁止覆盖为当前提交 {}",
            tag_name, existing, current
        ))
    }
}

fn create_immutable_tag(project_path: &str, tag_name: &str) -> Result<(), String> {
    run_git(
        project_path,
        &["tag", tag_name],
        &format!("创建 Git 标签 {} 失败", tag_name),
    )?;
    Ok(())
}

/// 中阶段节点只创建空提交和不可覆盖标签；调用前工作区必须干净。
pub(crate) async fn git_save_node(
    project_path: String,
    version: String,
    title: String,
) -> Result<String, String> {
    ensure_clean_workspace(&project_path)?;
    let tag_name = format!("metheus/{}", version);
    if ensure_tag_available(&project_path, &tag_name)? {
        return Ok(tag_name);
    }

    let commit_message = format!("【弥】节点 {}: {}", version, title);
    run_git(
        &project_path,
        &["commit", "--allow-empty", "-m", &commit_message],
        "创建中阶段节点提交失败",
    )?;
    create_immutable_tag(&project_path, &tag_name)?;
    Ok(tag_name)
}

/// 小阶段确认只暂存计划授权路径和经过原内容校验的系统生成文件，并创建不可覆盖标签。
pub(crate) async fn git_save_subtask(
    project_path: String,
    subtask_index: u32,
    mid_stage_version: String,
    subtask_title: String,
    authorized_paths: Vec<String>,
    generated_file: Option<GeneratedFileUpdate>,
) -> Result<String, String> {
    if authorized_paths.is_empty() {
        return Err("小阶段授权文件范围为空，拒绝提交".to_string());
    }
    let tag_name = format!("metheus/auto/{}/task-{}", mid_stage_version, subtask_index);
    if ensure_tag_available(&project_path, &tag_name)? {
        return Ok(tag_name);
    }
    ensure_only_authorized_changes(&project_path, &authorized_paths)?;

    let generated_file = generated_file.filter(GeneratedFileUpdate::changed);
    let mut commit_paths = authorized_paths.clone();
    if let Some(update) = generated_file.as_ref() {
        let generated_path = Path::new(&project_path).join(&update.relative_path);
        let current_content = fs::read_to_string(&generated_path).map_err(|error| {
            format!("读取系统生成文件 {} 失败：{}", update.relative_path, error)
        })?;
        if current_content != update.original_content {
            return Err(format!(
                "系统生成文件 {} 在确认期间发生变化，拒绝覆盖",
                update.relative_path
            ));
        }
        atomic_write_text(&generated_path, &update.updated_content)?;
        if !commit_paths.contains(&update.relative_path) {
            commit_paths.push(update.relative_path.clone());
        }
    }

    let mut committed = false;
    let save_result = (|| {
        ensure_only_authorized_changes(&project_path, &commit_paths)?;

        let pathspecs: Vec<String> = commit_paths
            .iter()
            .map(|path| format!(":(literal){}", path))
            .collect();
        let mut add_args = vec!["add", "-A", "--"];
        add_args.extend(pathspecs.iter().map(String::as_str));
        run_git(&project_path, &add_args, "暂存小阶段授权文件失败")?;

        let staged = run_git(
            &project_path,
            &["diff", "--cached", "--name-only", "-z"],
            "读取暂存区失败",
        )?;
        let authorized: BTreeSet<&str> = commit_paths.iter().map(String::as_str).collect();
        let outside: Vec<String> = staged
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).to_string())
            .filter(|path| !authorized.contains(path.as_str()))
            .collect();
        if !outside.is_empty() {
            return Err(format!(
                "暂存区包含计划范围外文件，拒绝提交：{}",
                outside.join("、")
            ));
        }

        let commit_message = format!(
            "【弥】小阶段 {}/{}：{}",
            subtask_index, mid_stage_version, subtask_title
        );
        run_git(
            &project_path,
            &["commit", "--allow-empty", "-m", &commit_message],
            "创建小阶段提交失败",
        )?;
        committed = true;
        create_immutable_tag(&project_path, &tag_name)?;
        ensure_clean_workspace(&project_path)?;
        Ok(tag_name.clone())
    })();

    if save_result.is_err() && !committed {
        if let Some(update) = generated_file.as_ref() {
            let generated_path = Path::new(&project_path).join(&update.relative_path);
            let pathspec = format!(":(literal){}", update.relative_path);
            let _ = run_git(
                &project_path,
                &["reset", "--quiet", "HEAD", "--", &pathspec],
                "恢复系统生成文件暂存状态失败",
            );
            if let Err(restore_error) = atomic_write_text(&generated_path, &update.original_content)
            {
                return Err(format!(
                    "{}；同时恢复 {} 失败：{}",
                    save_result.unwrap_err(),
                    update.relative_path,
                    restore_error
                ));
            }
        }
    }

    save_result
}

/// 手工回退只接受干净工作区，不自动 stash 或丢弃用户变更。
pub(crate) fn git_reset_to_tag_clean(project_path: &str, tag_name: &str) -> Result<(), String> {
    ensure_clean_workspace(project_path)?;
    run_git(
        project_path,
        &["rev-parse", "--verify", &format!("{}^{{commit}}", tag_name)],
        &format!("回退目标 {} 不存在", tag_name),
    )?;
    run_git(
        project_path,
        &["reset", "--hard", tag_name],
        &format!("回退到 {} 失败", tag_name),
    )?;
    ensure_clean_workspace(project_path)
}

pub(crate) fn delete_tags(project_path: &str, tags: &[String]) -> Result<(), String> {
    for tag in tags {
        if tag.is_empty() || tag_target(project_path, tag)?.is_none() {
            continue;
        }
        run_git(
            project_path,
            &["tag", "-d", tag],
            &format!("删除废弃 Git 标签 {} 失败", tag),
        )?;
    }
    Ok(())
}

/// 返回项目状态树中记录的 Metheus 标签。
#[tauri::command]
pub(crate) async fn get_git_tags_summary(
    project_name: String,
) -> Result<project::GitTagTree, String> {
    let proj = crate::load_project(&project_name)?;
    let milestones = proj
        .milestones
        .iter()
        .map(|milestone| project::MilestoneTagNode {
            milestone_id: milestone.id.clone(),
            milestone_title: milestone.title.clone(),
            milestone_version: milestone.version.clone(),
            milestone_status: format!("{:?}", milestone.status),
            mid_stages: milestone
                .mid_stages
                .iter()
                .map(|mid_stage| project::MidStageTagNode {
                    mid_stage_id: mid_stage.id.clone(),
                    mid_stage_title: mid_stage.title.clone(),
                    mid_stage_version: mid_stage.version.clone(),
                    mid_stage_tag: mid_stage.git_tag.clone(),
                    mid_stage_status: format!("{:?}", mid_stage.status),
                    subtasks: mid_stage
                        .subtasks
                        .iter()
                        .enumerate()
                        .map(|(index, subtask)| project::SubtaskTagNode {
                            subtask_id: subtask.id.clone(),
                            subtask_title: subtask.title.clone(),
                            subtask_index: (index + 1) as u32,
                            subtask_tag: subtask.auto_tag.clone().unwrap_or_default(),
                            subtask_status: format!("{:?}", subtask.status),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    Ok(project::GitTagTree { milestones })
}

/// 返回 staged、unstaged 和 untracked 状态；diff 内容覆盖已跟踪变更。
#[tauri::command]
pub(crate) async fn get_current_diff(project_path: String) -> Result<String, String> {
    if !std::path::Path::new(&project_path).join(".git").exists() {
        return Ok(String::new());
    }
    let status = run_git(
        &project_path,
        &["status", "--short", "--untracked-files=all"],
        "读取 Git 变更状态失败",
    )?;
    let diff = run_git(&project_path, &["diff", "HEAD", "--"], "读取 Git diff 失败")?;
    let status = String::from_utf8_lossy(&status).trim().to_string();
    let diff = String::from_utf8_lossy(&diff).trim().to_string();
    if status.is_empty() && diff.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("工作区状态：\n{}\n\n变更内容：\n{}", status, diff))
    }
}

#[tauri::command]
pub(crate) async fn get_change_history(
    project_name: String,
) -> Result<Vec<project::ChangeHistoryEntry>, String> {
    Ok(crate::load_project(&project_name)?.change_history)
}

pub(crate) fn save_tag_to_mid_stage(
    project_id: &str,
    mid_stage_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    let mut project = crate::load_project(project_id)?;
    let mid_stage = project
        .milestones
        .iter_mut()
        .flat_map(|milestone| milestone.mid_stages.iter_mut())
        .find(|mid_stage| mid_stage.id == mid_stage_id)
        .ok_or_else(|| format!("未找到中阶段: {}", mid_stage_id))?;
    mid_stage.git_tag = tag_name.to_string();
    crate::save_project(&project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("metheus-git-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            let repo = Self(path);
            repo.git(&["init", "--quiet"]);
            repo.git(&["config", "user.name", "Metheus Test"]);
            repo.git(&["config", "user.email", "metheus-test@example.invalid"]);
            std::fs::write(repo.0.join("tracked.txt"), "baseline\n").unwrap();
            repo.git(&["add", "tracked.txt"]);
            repo.git(&["commit", "--quiet", "-m", "baseline"]);
            repo
        }

        fn git(&self, args: &[&str]) -> String {
            String::from_utf8_lossy(
                &run_git(self.0.to_str().unwrap(), args, "测试 Git 命令失败").unwrap(),
            )
            .trim()
            .to_string()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn subtask_commit_rejects_outside_changes_and_never_overwrites_tag() {
        let repo = TempRepo::new();
        std::fs::write(repo.0.join("tracked.txt"), "changed\n").unwrap();
        std::fs::write(repo.0.join("outside.txt"), "outside\n").unwrap();
        let path = repo.0.to_string_lossy().to_string();
        let rejected = git_save_subtask(
            path.clone(),
            1,
            "v0.1.1".to_string(),
            "测试".to_string(),
            vec!["tracked.txt".to_string()],
            None,
        )
        .await;
        assert!(rejected.is_err());

        std::fs::remove_file(repo.0.join("outside.txt")).unwrap();
        let tag = git_save_subtask(
            path.clone(),
            1,
            "v0.1.1".to_string(),
            "测试".to_string(),
            vec!["tracked.txt".to_string()],
            None,
        )
        .await
        .unwrap();
        let original = repo.git(&["rev-parse", &tag]);
        repo.git(&["commit", "--allow-empty", "-m", "later"]);
        assert!(create_immutable_tag(&path, &tag).is_err());
        assert_eq!(repo.git(&["rev-parse", &tag]), original);
    }

    #[tokio::test]
    async fn subtask_commit_includes_generated_constitution_and_leaves_workspace_clean() {
        let repo = TempRepo::new();
        let original_constitution = "# Constitution\n\n## 第 2 部分\n待更新\n";
        let updated_constitution =
            "# Constitution\n\n## 第 2 部分\n\n### 项目结构\n- `index.html`\n";
        std::fs::write(repo.0.join("CONSTITUTION.md"), original_constitution).unwrap();
        repo.git(&["add", "CONSTITUTION.md"]);
        repo.git(&["commit", "--quiet", "-m", "constitution baseline"]);
        std::fs::write(repo.0.join("index.html"), "<main>ready</main>\n").unwrap();

        let path = repo.0.to_string_lossy().to_string();
        let authorized = vec!["index.html".to_string()];
        let diff = capture_authorized_diff(&path, &authorized).unwrap();
        assert!(diff.contains("new file mode"));
        assert!(diff.contains("index.html"));
        assert!(repo.git(&["status", "--short"]).contains("index.html"));

        let tag = git_save_subtask(
            path,
            2,
            "v0.1.1".to_string(),
            "HTML 与宪法同步".to_string(),
            authorized,
            Some(GeneratedFileUpdate::constitution(
                original_constitution.to_string(),
                updated_constitution.to_string(),
            )),
        )
        .await
        .unwrap();

        assert!(repo.git(&["status", "--short"]).is_empty());
        let committed_files = repo.git(&["show", "--format=", "--name-only", &tag]);
        assert!(committed_files.contains("index.html"));
        assert!(committed_files.contains("CONSTITUTION.md"));
        assert_eq!(
            std::fs::read_to_string(repo.0.join("CONSTITUTION.md")).unwrap(),
            updated_constitution
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_commit_restores_generated_constitution() {
        use std::os::unix::fs::PermissionsExt;

        let repo = TempRepo::new();
        let original_constitution = "# Constitution\n\n## 第 2 部分\n原始内容\n";
        let updated_constitution = "# Constitution\n\n## 第 2 部分\n更新内容\n";
        std::fs::write(repo.0.join("CONSTITUTION.md"), original_constitution).unwrap();
        repo.git(&["add", "CONSTITUTION.md"]);
        repo.git(&["commit", "--quiet", "-m", "constitution baseline"]);
        std::fs::write(repo.0.join("tracked.txt"), "task change\n").unwrap();

        let hook_path = repo.0.join(".git/hooks/pre-commit");
        std::fs::write(&hook_path, "#!/bin/sh\nexit 1\n").unwrap();
        let mut permissions = std::fs::metadata(&hook_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook_path, permissions).unwrap();

        let result = git_save_subtask(
            repo.0.to_string_lossy().to_string(),
            3,
            "v0.1.1".to_string(),
            "失败恢复".to_string(),
            vec!["tracked.txt".to_string()],
            Some(GeneratedFileUpdate::constitution(
                original_constitution.to_string(),
                updated_constitution.to_string(),
            )),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(repo.0.join("CONSTITUTION.md")).unwrap(),
            original_constitution
        );
        assert!(!repo.git(&["status", "--short"]).contains("CONSTITUTION.md"));
    }

    #[test]
    fn manual_reset_rejects_dirty_workspace() {
        let repo = TempRepo::new();
        let target = repo.git(&["rev-parse", "HEAD"]);
        std::fs::write(repo.0.join("tracked.txt"), "dirty\n").unwrap();
        assert!(git_reset_to_tag_clean(repo.0.to_str().unwrap(), &target).is_err());
    }
}
