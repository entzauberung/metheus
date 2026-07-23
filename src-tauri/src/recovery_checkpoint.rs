use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointManifest {
    project_path: String,
    entries: Vec<CheckpointEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointEntry {
    relative_path: String,
    existed: bool,
}

fn root() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".metheus").join("recovery-checkpoints"))
        .ok_or_else(|| "无法获取恢复检查点目录。".to_string())
}

fn checkpoint_path(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-') {
        return Err("恢复检查点标识无效。".to_string());
    }
    Ok(root()?.join(id))
}

pub(crate) fn create(project_path: &str, paths: &[String]) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let directory = checkpoint_path(&id)?;
    let files = directory.join("files");
    std::fs::create_dir_all(&files).map_err(|error| format!("创建恢复检查点失败：{}", error))?;
    let mut entries = Vec::new();
    for relative in paths {
        let source = Path::new(project_path).join(relative);
        let existed = source.is_file();
        if existed {
            let target = files.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("创建检查点目录失败：{}", error))?;
            }
            std::fs::copy(&source, &target)
                .map_err(|error| format!("保存检查点文件 {} 失败：{}", relative, error))?;
        }
        entries.push(CheckpointEntry {
            relative_path: relative.clone(),
            existed,
        });
    }
    let manifest = CheckpointManifest {
        project_path: project_path.to_string(),
        entries,
    };
    let data = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("序列化恢复检查点失败：{}", error))?;
    std::fs::write(directory.join("manifest.json"), data)
        .map_err(|error| format!("写入恢复检查点失败：{}", error))?;
    Ok(id)
}

pub(crate) fn restore(id: &str) -> Result<(), String> {
    let directory = checkpoint_path(id)?;
    let data = std::fs::read(directory.join("manifest.json"))
        .map_err(|error| format!("读取恢复检查点失败：{}", error))?;
    let manifest: CheckpointManifest =
        serde_json::from_slice(&data).map_err(|error| format!("解析恢复检查点失败：{}", error))?;
    for entry in &manifest.entries {
        let target = Path::new(&manifest.project_path).join(&entry.relative_path);
        if entry.existed {
            let source = directory.join("files").join(&entry.relative_path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("恢复检查点目录失败：{}", error))?;
            }
            std::fs::copy(&source, &target).map_err(|error| {
                format!("恢复检查点文件 {} 失败：{}", entry.relative_path, error)
            })?;
        } else if target.exists() {
            std::fs::remove_file(&target).map_err(|error| {
                format!("移除本轮新增文件 {} 失败：{}", entry.relative_path, error)
            })?;
        }
    }
    discard(id)
}

pub(crate) fn discard(id: &str) -> Result<(), String> {
    let directory = checkpoint_path(id)?;
    if directory.exists() {
        std::fs::remove_dir_all(directory)
            .map_err(|error| format!("清理恢复检查点失败：{}", error))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_modified_and_new_files() -> Result<(), String> {
        let project =
            std::env::temp_dir().join(format!("metheus-checkpoint-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&project).map_err(|error| error.to_string())?;
        std::fs::write(project.join("existing.txt"), "before")
            .map_err(|error| error.to_string())?;
        let id = create(
            &project.to_string_lossy(),
            &["existing.txt".to_string(), "new.txt".to_string()],
        )?;
        std::fs::write(project.join("existing.txt"), "after").map_err(|error| error.to_string())?;
        std::fs::write(project.join("new.txt"), "new").map_err(|error| error.to_string())?;
        restore(&id)?;
        assert_eq!(
            std::fs::read_to_string(project.join("existing.txt")).unwrap(),
            "before"
        );
        assert!(!project.join("new.txt").exists());
        std::fs::remove_dir_all(project).map_err(|error| error.to_string())?;
        Ok(())
    }
}
