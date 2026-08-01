use crate::project;
use sha2::{Digest, Sha256};
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

pub(crate) struct GitRecoveryCommitImpact {
    pub current_head: String,
    pub target_commit: String,
    pub changed_since_target: Vec<String>,
}

/// Read-only committed-file impact for a future hard reset.
pub(crate) fn preview_reset_impact(
    project_path: &str,
    target: &str,
) -> Result<GitRecoveryCommitImpact, String> {
    let current_head = head(project_path)?;
    let target_commit = run_git(
        project_path,
        &["rev-parse", "--verify", &format!("{}^{{commit}}", target)],
        "校验执行恢复基线失败",
    )?;
    let target_commit = String::from_utf8_lossy(&target_commit).trim().to_string();
    let output = run_git(
        project_path,
        &["diff", "--name-only", "-z", &target_commit, &current_head],
        "读取执行基线影响失败",
    )?;
    let mut changed_since_target = output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).to_string())
        .collect::<Vec<_>>();
    changed_since_target.sort();
    changed_since_target.dedup();
    Ok(GitRecoveryCommitImpact {
        current_head,
        target_commit,
        changed_since_target,
    })
}

/// Hashes the exact tracked diff and untracked file bytes used by a recovery preview.
/// Paths are sorted so repeated reads of the same workspace produce the same value.
pub(crate) fn recovery_workspace_fingerprint(
    project_path: &str,
    untracked_files: &[String],
) -> Result<String, String> {
    let tracked_diff = run_git(
        project_path,
        &["diff", "HEAD", "--binary", "--no-ext-diff"],
        "读取执行恢复工作区差异失败",
    )?;
    let mut hasher = Sha256::new();
    hasher.update(b"tracked\0");
    hasher.update(&tracked_diff);
    let mut untracked = untracked_files.to_vec();
    untracked.sort();
    untracked.dedup();
    for relative in untracked {
        hasher.update(b"untracked\0");
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        let path = Path::new(project_path).join(&relative);
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "读取恢复预览中的未跟踪文件 {} 失败：{}",
                path.display(),
                error
            )
        })?;
        hasher.update(&bytes);
        hasher.update(b"\0");
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
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

pub(crate) fn tag_target(project_path: &str, tag_name: &str) -> Result<Option<String>, String> {
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

fn tag_identity_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' {
            encoded.push(byte as char);
        } else {
            encoded.push('_');
            encoded.push_str(&format!("{:02x}", byte));
        }
    }
    if encoded.is_empty() {
        "empty".to_string()
    } else {
        encoded
    }
}

pub(crate) fn subtask_v2_tag(
    milestone_id: &str,
    mid_stage_id: &str,
    subtask_id: &str,
    transaction_id: &str,
) -> String {
    format!(
        "metheus/v2/subtask/{}/{}/{}/{}",
        tag_identity_component(milestone_id),
        tag_identity_component(mid_stage_id),
        tag_identity_component(subtask_id),
        tag_identity_component(transaction_id),
    )
}

pub(crate) fn node_v2_tag(milestone_id: &str, mid_stage_id: &str, transaction_id: &str) -> String {
    format!(
        "metheus/v2/node/{}/{}/{}",
        tag_identity_component(milestone_id),
        tag_identity_component(mid_stage_id),
        tag_identity_component(transaction_id),
    )
}

fn transaction_trailer(transaction_id: &str) -> String {
    format!("Metheus-Confirmation: {}", transaction_id)
}

fn node_transaction_trailer(transaction_id: &str) -> String {
    format!("Metheus-Node-Confirmation: {}", transaction_id)
}

fn commit_has_trailer(project_path: &str, commit: &str, trailer: &str) -> Result<bool, String> {
    let message = run_git(
        project_path,
        &["show", "-s", "--format=%B", commit],
        "读取 Git 确认提交信息失败",
    )?;
    Ok(String::from_utf8_lossy(&message)
        .lines()
        .any(|line| line.trim() == trailer))
}

fn commit_has_transaction(
    project_path: &str,
    commit: &str,
    transaction_id: &str,
) -> Result<bool, String> {
    commit_has_trailer(project_path, commit, &transaction_trailer(transaction_id))
}

fn commit_has_node_transaction(
    project_path: &str,
    commit: &str,
    transaction_id: &str,
) -> Result<bool, String> {
    commit_has_trailer(
        project_path,
        commit,
        &node_transaction_trailer(transaction_id),
    )
}

fn create_immutable_tag_at(project_path: &str, tag_name: &str, commit: &str) -> Result<(), String> {
    run_git(
        project_path,
        &["tag", tag_name, commit],
        &format!("创建 Git 标签 {} 失败", tag_name),
    )?;
    Ok(())
}

#[cfg(test)]
fn create_immutable_tag(project_path: &str, tag_name: &str) -> Result<(), String> {
    let current = head(project_path)?;
    create_immutable_tag_at(project_path, tag_name, &current)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitConfirmationError {
    pub kind: project::GitConfirmationFailureKind,
    pub message: String,
}

impl GitConfirmationError {
    fn new(kind: project::GitConfirmationFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitSaveProgress {
    CommitCreated { commit: String, tag: String },
    TagCreated { commit: String, tag: String },
}

/// 中阶段节点只创建空提交和不可覆盖标签；调用前工作区必须干净。
pub(crate) async fn git_save_node(
    project_path: String,
    milestone_id: String,
    mid_stage_id: String,
    transaction_id: String,
    version: String,
    title: String,
) -> Result<String, String> {
    ensure_clean_workspace(&project_path)?;
    let tag_name = node_v2_tag(&milestone_id, &mid_stage_id, &transaction_id);
    if let Some(existing) = tag_target(&project_path, &tag_name)? {
        if commit_has_node_transaction(&project_path, &existing, &transaction_id)? {
            return Ok(tag_name);
        }
        return Err(format!(
            "Git 标签 {} 已指向其他确认事务的提交 {}，禁止覆盖",
            tag_name, existing
        ));
    }
    let current = head(&project_path)?;
    if commit_has_node_transaction(&project_path, &current, &transaction_id)? {
        create_immutable_tag_at(&project_path, &tag_name, &current)?;
        return Ok(tag_name);
    }

    let commit_message = format!(
        "【弥】节点 {}: {}\n\n{}",
        version,
        title,
        node_transaction_trailer(&transaction_id)
    );
    run_git(
        &project_path,
        &["commit", "--allow-empty", "-m", &commit_message],
        "创建中阶段节点提交失败",
    )?;
    let commit = head(&project_path)?;
    create_immutable_tag_at(&project_path, &tag_name, &commit)?;
    Ok(tag_name)
}

/// 小阶段确认事务一次只推进一个持久化阶段：先提交，再由调用方落盘后补建标签。
pub(crate) async fn git_save_subtask(
    project_path: String,
    milestone_id: String,
    mid_stage_id: String,
    subtask_id: String,
    transaction_id: String,
    subtask_index: u32,
    mid_stage_version: String,
    subtask_title: String,
    authorized_paths: Vec<String>,
    generated_file: Option<GeneratedFileUpdate>,
    confirmation_phase: project::ConfirmationPhase,
    confirmation_commit: String,
) -> Result<GitSaveProgress, GitConfirmationError> {
    if authorized_paths.is_empty() {
        return Err(GitConfirmationError::new(
            project::GitConfirmationFailureKind::ScopeViolation,
            "小阶段授权文件范围为空，拒绝提交",
        ));
    }
    if transaction_id.is_empty() {
        return Err(GitConfirmationError::new(
            project::GitConfirmationFailureKind::CommitFailed,
            "确认事务标识为空，拒绝执行 Git 保存",
        ));
    }
    let tag_name = subtask_v2_tag(&milestone_id, &mid_stage_id, &subtask_id, &transaction_id);

    if let Some(existing) = tag_target(&project_path, &tag_name).map_err(|message| {
        GitConfirmationError::new(project::GitConfirmationFailureKind::TagFailed, message)
    })? {
        if commit_has_transaction(&project_path, &existing, &transaction_id).map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::TagFailed, message)
        })? {
            return Ok(GitSaveProgress::TagCreated {
                commit: existing,
                tag: tag_name,
            });
        }
        return Err(GitConfirmationError::new(
            project::GitConfirmationFailureKind::V2TagIntegrityConflict,
            format!(
                "Git 标签 {} 已指向其他确认事务的提交 {}，禁止覆盖",
                tag_name, existing
            ),
        ));
    }

    if matches!(
        confirmation_phase,
        project::ConfirmationPhase::CommitCreated
            | project::ConfirmationPhase::TagCreated
            | project::ConfirmationPhase::ProjectFinalizing
    ) || !confirmation_commit.is_empty()
    {
        let commit = if confirmation_commit.is_empty() {
            head(&project_path).map_err(|message| {
                GitConfirmationError::new(project::GitConfirmationFailureKind::TagFailed, message)
            })?
        } else {
            confirmation_commit
        };
        let belongs_to_transaction =
            commit_has_transaction(&project_path, &commit, &transaction_id).map_err(|message| {
                GitConfirmationError::new(project::GitConfirmationFailureKind::TagFailed, message)
            })?;
        if !belongs_to_transaction {
            return Err(GitConfirmationError::new(
                project::GitConfirmationFailureKind::V2TagIntegrityConflict,
                format!("提交 {} 不属于当前确认事务，拒绝创建标签", commit),
            ));
        }
        create_immutable_tag_at(&project_path, &tag_name, &commit).map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::TagFailed, message)
        })?;
        return Ok(GitSaveProgress::TagCreated {
            commit,
            tag: tag_name,
        });
    }

    let current = head(&project_path).map_err(|message| {
        GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
    })?;
    if commit_has_transaction(&project_path, &current, &transaction_id).map_err(|message| {
        GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
    })? {
        return Ok(GitSaveProgress::CommitCreated {
            commit: current,
            tag: tag_name,
        });
    }

    ensure_only_authorized_changes(&project_path, &authorized_paths).map_err(|message| {
        GitConfirmationError::new(project::GitConfirmationFailureKind::ScopeViolation, message)
    })?;

    let generated_file = generated_file.filter(GeneratedFileUpdate::changed);
    let mut commit_paths = authorized_paths.clone();
    if let Some(update) = generated_file.as_ref() {
        let generated_path = Path::new(&project_path).join(&update.relative_path);
        let current_content = fs::read_to_string(&generated_path).map_err(|error| {
            GitConfirmationError::new(
                project::GitConfirmationFailureKind::CommitFailed,
                format!("读取系统生成文件 {} 失败：{}", update.relative_path, error),
            )
        })?;
        if current_content != update.original_content {
            return Err(GitConfirmationError::new(
                project::GitConfirmationFailureKind::ScopeViolation,
                format!(
                    "系统生成文件 {} 在确认期间发生变化，拒绝覆盖",
                    update.relative_path
                ),
            ));
        }
        atomic_write_text(&generated_path, &update.updated_content).map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
        })?;
        if !commit_paths.contains(&update.relative_path) {
            commit_paths.push(update.relative_path.clone());
        }
    }

    let mut committed = false;
    let save_result: Result<GitSaveProgress, GitConfirmationError> = (|| {
        ensure_only_authorized_changes(&project_path, &commit_paths).map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::ScopeViolation, message)
        })?;

        let pathspecs: Vec<String> = commit_paths
            .iter()
            .map(|path| format!(":(literal){}", path))
            .collect();
        let mut add_args = vec!["add", "-A", "--"];
        add_args.extend(pathspecs.iter().map(String::as_str));
        run_git(&project_path, &add_args, "暂存小阶段授权文件失败").map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
        })?;

        let staged = run_git(
            &project_path,
            &["diff", "--cached", "--name-only", "-z"],
            "读取暂存区失败",
        )
        .map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
        })?;
        let authorized: BTreeSet<&str> = commit_paths.iter().map(String::as_str).collect();
        let outside: Vec<String> = staged
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).to_string())
            .filter(|path| !authorized.contains(path.as_str()))
            .collect();
        if !outside.is_empty() {
            return Err(GitConfirmationError::new(
                project::GitConfirmationFailureKind::ScopeViolation,
                format!("暂存区包含计划范围外文件，拒绝提交：{}", outside.join("、")),
            ));
        }

        let commit_message = format!(
            "【弥】小阶段 {}/{}：{}\n\n{}",
            subtask_index,
            mid_stage_version,
            subtask_title,
            transaction_trailer(&transaction_id),
        );
        run_git(
            &project_path,
            &["commit", "--allow-empty", "-m", &commit_message],
            "创建小阶段提交失败",
        )
        .map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
        })?;
        committed = true;
        let commit = head(&project_path).map_err(|message| {
            GitConfirmationError::new(project::GitConfirmationFailureKind::CommitFailed, message)
        })?;
        Ok(GitSaveProgress::CommitCreated {
            commit,
            tag: tag_name.clone(),
        })
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
                let original_error = save_result.unwrap_err();
                return Err(GitConfirmationError::new(
                    original_error.kind,
                    format!(
                        "{}；同时恢复 {} 失败：{}",
                        original_error.message, update.relative_path, restore_error
                    ),
                ));
            }
        }
    }

    save_result
}

/// 读取指定确认提交中仅属于任务授权路径的 diff，用于项目收口中断后的幂等恢复。
pub(crate) fn capture_commit_diff(
    project_path: &str,
    commit: &str,
    authorized_paths: &[String],
) -> Result<String, String> {
    let pathspecs: Vec<String> = authorized_paths
        .iter()
        .map(|path| format!(":(literal){}", path))
        .collect();
    let mut args = vec!["show", "--format=", "--binary", commit, "--"];
    args.extend(pathspecs.iter().map(String::as_str));
    run_git(project_path, &args, "读取确认提交差异失败")
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
}

pub(crate) fn commit_changed_path(
    project_path: &str,
    commit: &str,
    relative_path: &str,
) -> Result<bool, String> {
    let pathspec = format!(":(literal){}", relative_path);
    let output = run_git(
        project_path,
        &[
            "show",
            "--format=",
            "--name-only",
            "-z",
            commit,
            "--",
            &pathspec,
        ],
        "读取确认提交文件列表失败",
    )?;
    Ok(output
        .split(|byte| *byte == 0)
        .any(|entry| entry == relative_path.as_bytes()))
}

/// 通过 Git 事实识别旧 V1 标签碰撞，不依赖历史错误文本。
pub(crate) fn is_legacy_v1_tag_conflict(
    project_path: &str,
    mid_stage_version: &str,
    subtask_index: u32,
    authorized_paths: &[String],
) -> Result<bool, String> {
    if authorized_paths.is_empty() {
        return Ok(false);
    }
    let legacy_tag = format!("metheus/auto/{}/task-{}", mid_stage_version, subtask_index);
    let Some(existing) = tag_target(project_path, &legacy_tag)? else {
        return Ok(false);
    };
    if existing == head(project_path)? {
        return Ok(false);
    }
    Ok(ensure_only_authorized_changes(project_path, authorized_paths).is_ok())
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

    async fn complete_git_confirmation(
        project_path: String,
        milestone_id: &str,
        mid_stage_id: &str,
        subtask_id: &str,
        transaction_id: &str,
        subtask_index: u32,
        title: &str,
        authorized_paths: Vec<String>,
        generated_file: Option<GeneratedFileUpdate>,
    ) -> Result<(String, String), GitConfirmationError> {
        let first = git_save_subtask(
            project_path.clone(),
            milestone_id.to_string(),
            mid_stage_id.to_string(),
            subtask_id.to_string(),
            transaction_id.to_string(),
            subtask_index,
            "v0.1.1".to_string(),
            title.to_string(),
            authorized_paths.clone(),
            generated_file,
            project::ConfirmationPhase::Preparing,
            String::new(),
        )
        .await?;
        match first {
            GitSaveProgress::TagCreated { commit, tag } => Ok((commit, tag)),
            GitSaveProgress::CommitCreated { commit, .. } => {
                match git_save_subtask(
                    project_path,
                    milestone_id.to_string(),
                    mid_stage_id.to_string(),
                    subtask_id.to_string(),
                    transaction_id.to_string(),
                    subtask_index,
                    "v0.1.1".to_string(),
                    title.to_string(),
                    authorized_paths,
                    None,
                    project::ConfirmationPhase::CommitCreated,
                    commit,
                )
                .await?
                {
                    GitSaveProgress::TagCreated { commit, tag } => Ok((commit, tag)),
                    GitSaveProgress::CommitCreated { .. } => {
                        unreachable!("提交阶段之后必须推进到标签阶段")
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn git_confirmation_rejects_outside_changes_and_never_overwrites_tag() {
        let repo = TempRepo::new();
        std::fs::write(repo.0.join("tracked.txt"), "changed\n").unwrap();
        std::fs::write(repo.0.join("outside.txt"), "outside\n").unwrap();
        let path = repo.0.to_string_lossy().to_string();
        let rejected = git_save_subtask(
            path.clone(),
            "milestone-a".to_string(),
            "mid-a".to_string(),
            "subtask-a".to_string(),
            "transaction-a".to_string(),
            1,
            "v0.1.1".to_string(),
            "测试".to_string(),
            vec!["tracked.txt".to_string()],
            None,
            project::ConfirmationPhase::Preparing,
            String::new(),
        )
        .await;
        assert!(rejected.is_err());

        std::fs::remove_file(repo.0.join("outside.txt")).unwrap();
        let (_, tag) = complete_git_confirmation(
            path.clone(),
            "milestone-a",
            "mid-a",
            "subtask-a",
            "transaction-a",
            1,
            "测试",
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
    async fn git_confirmation_includes_generated_constitution_and_leaves_workspace_clean() {
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

        let (_, tag) = complete_git_confirmation(
            path,
            "milestone-a",
            "mid-a",
            "subtask-b",
            "transaction-b",
            2,
            "HTML 与宪法同步",
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
    async fn git_confirmation_failed_commit_restores_generated_constitution() {
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
            "milestone-a".to_string(),
            "mid-a".to_string(),
            "subtask-c".to_string(),
            "transaction-c".to_string(),
            3,
            "v0.1.1".to_string(),
            "失败恢复".to_string(),
            vec!["tracked.txt".to_string()],
            Some(GeneratedFileUpdate::constitution(
                original_constitution.to_string(),
                updated_constitution.to_string(),
            )),
            project::ConfirmationPhase::Preparing,
            String::new(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(repo.0.join("CONSTITUTION.md")).unwrap(),
            original_constitution
        );
        assert!(!repo.git(&["status", "--short"]).contains("CONSTITUTION.md"));
    }

    #[tokio::test]
    async fn git_confirmation_v2_identity_does_not_collide_across_milestones() {
        let repo = TempRepo::new();
        let path = repo.0.to_string_lossy().to_string();
        std::fs::write(repo.0.join("tracked.txt"), "milestone one\n").unwrap();
        let (_, first_tag) = complete_git_confirmation(
            path.clone(),
            "milestone-one",
            "mid-shared",
            "subtask-shared",
            "transaction-one",
            1,
            "共享序号",
            vec!["tracked.txt".to_string()],
            None,
        )
        .await
        .unwrap();

        std::fs::write(repo.0.join("tracked.txt"), "milestone two\n").unwrap();
        let (_, second_tag) = complete_git_confirmation(
            path,
            "milestone-two",
            "mid-shared",
            "subtask-shared",
            "transaction-two",
            1,
            "共享序号",
            vec!["tracked.txt".to_string()],
            None,
        )
        .await
        .unwrap();

        assert_ne!(first_tag, second_tag);
        assert!(first_tag.contains("milestone-one"));
        assert!(second_tag.contains("milestone-two"));
    }

    #[tokio::test]
    async fn git_confirmation_rejects_v2_tag_owned_by_another_transaction() {
        let repo = TempRepo::new();
        let path = repo.0.to_string_lossy().to_string();
        let transaction_id = "transaction-integrity-conflict";
        let tag = subtask_v2_tag("milestone-a", "mid-a", "subtask-a", transaction_id);
        let unrelated_commit = repo.git(&["rev-parse", "HEAD"]);
        create_immutable_tag_at(&path, &tag, &unrelated_commit).unwrap();
        std::fs::write(repo.0.join("tracked.txt"), "approved task change\n").unwrap();
        let commit_count = repo.git(&["rev-list", "--count", "HEAD"]);

        let error = git_save_subtask(
            path.clone(),
            "milestone-a".to_string(),
            "mid-a".to_string(),
            "subtask-a".to_string(),
            transaction_id.to_string(),
            1,
            "v0.1.1".to_string(),
            "完整性冲突".to_string(),
            vec!["tracked.txt".to_string()],
            None,
            project::ConfirmationPhase::Preparing,
            String::new(),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.kind,
            project::GitConfirmationFailureKind::V2TagIntegrityConflict
        );
        assert_eq!(tag_target(&path, &tag).unwrap(), Some(unrelated_commit));
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), commit_count);
        assert!(repo.git(&["status", "--short"]).contains("tracked.txt"));
    }

    #[tokio::test]
    async fn git_confirmation_retries_tag_without_creating_another_commit() {
        let repo = TempRepo::new();
        let path = repo.0.to_string_lossy().to_string();
        std::fs::write(repo.0.join("tracked.txt"), "confirmed\n").unwrap();
        let transaction_id = "transaction-retry";
        let first = git_save_subtask(
            path.clone(),
            "milestone-a".to_string(),
            "mid-a".to_string(),
            "subtask-retry".to_string(),
            transaction_id.to_string(),
            1,
            "v0.1.1".to_string(),
            "标签重试".to_string(),
            vec!["tracked.txt".to_string()],
            None,
            project::ConfirmationPhase::Preparing,
            String::new(),
        )
        .await
        .unwrap();
        let (commit, tag) = match first {
            GitSaveProgress::CommitCreated { commit, tag } => (commit, tag),
            GitSaveProgress::TagCreated { .. } => panic!("首次调用不应越过提交落盘边界"),
        };
        let commit_count = repo.git(&["rev-list", "--count", "HEAD"]);

        let lock_path = repo.0.join(".git/refs/tags").join(format!("{}.lock", tag));
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        std::fs::write(&lock_path, "locked").unwrap();
        let failed = git_save_subtask(
            path.clone(),
            "milestone-a".to_string(),
            "mid-a".to_string(),
            "subtask-retry".to_string(),
            transaction_id.to_string(),
            1,
            "v0.1.1".to_string(),
            "标签重试".to_string(),
            vec!["tracked.txt".to_string()],
            None,
            project::ConfirmationPhase::CommitCreated,
            commit.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(failed.kind, project::GitConfirmationFailureKind::TagFailed);

        std::fs::remove_file(lock_path).unwrap();
        let completed = git_save_subtask(
            path,
            "milestone-a".to_string(),
            "mid-a".to_string(),
            "subtask-retry".to_string(),
            transaction_id.to_string(),
            1,
            "v0.1.1".to_string(),
            "标签重试".to_string(),
            vec!["tracked.txt".to_string()],
            None,
            project::ConfirmationPhase::CommitCreated,
            commit.clone(),
        )
        .await
        .unwrap();
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), commit_count);
        assert_eq!(completed, GitSaveProgress::TagCreated { commit, tag });
    }

    #[tokio::test]
    async fn git_confirmation_node_v2_identity_is_idempotent_and_entity_scoped() {
        let repo = TempRepo::new();
        let path = repo.0.to_string_lossy().to_string();
        let first = git_save_node(
            path.clone(),
            "milestone-one".to_string(),
            "mid-shared".to_string(),
            "transaction-node-one".to_string(),
            "v0.1.1".to_string(),
            "共享节点".to_string(),
        )
        .await
        .unwrap();
        let after_first = repo.git(&["rev-list", "--count", "HEAD"]);
        let repeated = git_save_node(
            path.clone(),
            "milestone-one".to_string(),
            "mid-shared".to_string(),
            "transaction-node-one".to_string(),
            "v0.1.1".to_string(),
            "共享节点".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(repeated, first);
        assert_eq!(repo.git(&["rev-list", "--count", "HEAD"]), after_first);

        let second = git_save_node(
            path,
            "milestone-two".to_string(),
            "mid-shared".to_string(),
            "transaction-node-two".to_string(),
            "v0.1.1".to_string(),
            "共享节点".to_string(),
        )
        .await
        .unwrap();
        assert_ne!(first, second);
        assert!(first.contains("milestone-one"));
        assert!(second.contains("milestone-two"));
    }

    #[test]
    fn manual_reset_rejects_dirty_workspace() {
        let repo = TempRepo::new();
        let target = repo.git(&["rev-parse", "HEAD"]);
        std::fs::write(repo.0.join("tracked.txt"), "dirty\n").unwrap();
        assert!(git_reset_to_tag_clean(repo.0.to_str().unwrap(), &target).is_err());
    }
}
