use crate::project;
use std::collections::BTreeSet;
use std::path::{Component, Path};

fn validate_relative_file_path(path: &str, field: &str, entity: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(format!("{}的 {} 包含空路径", entity, field));
    }
    if trimmed != path {
        return Err(format!(
            "{}的 {} 路径前后不能包含空白：{}",
            entity, field, path
        ));
    }
    if trimmed.contains('\\') {
        return Err(format!(
            "{}的 {} 必须使用 / 分隔相对路径：{}",
            entity, field, path
        ));
    }
    if trimmed.ends_with('/') || trimmed.contains(['*', '?', '[', ']']) {
        return Err(format!(
            "{}的 {} 必须是精确文件路径，不能使用目录或通配符：{}",
            entity, field, path
        ));
    }

    let parsed = Path::new(trimmed);
    if parsed.is_absolute() {
        return Err(format!("{}的 {} 不能使用绝对路径：{}", entity, field, path));
    }

    let mut normalized = Vec::new();
    for component in parsed.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| format!("{}的 {} 不是有效 UTF-8 路径", entity, field))?;
                if normalized.is_empty() && part.ends_with(':') {
                    return Err(format!(
                        "{}的 {} 不能使用 Windows 绝对路径：{}",
                        entity, field, path
                    ));
                }
                normalized.push(part);
            }
            Component::CurDir => {
                return Err(format!("{}的 {} 不能包含 .：{}", entity, field, path));
            }
            Component::ParentDir => {
                return Err(format!("{}的 {} 不能包含 ..：{}", entity, field, path));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{}的 {} 必须是项目内相对路径：{}",
                    entity, field, path
                ));
            }
        }
    }
    if normalized.is_empty() {
        return Err(format!("{}的 {} 不能指向项目根目录", entity, field));
    }
    let normalized = normalized.join("/");
    if normalized != trimmed {
        return Err(format!(
            "{}的 {} 不是规范化相对路径：{}",
            entity, field, path
        ));
    }
    Ok(normalized)
}

fn validate_path_list(
    paths: &[String],
    field: &str,
    entity: &str,
    required: bool,
) -> Result<Vec<String>, String> {
    if required && paths.is_empty() {
        return Err(format!("{}的 {} 不能为空", entity, field));
    }

    let mut result = Vec::with_capacity(paths.len());
    let mut seen = BTreeSet::new();
    for path in paths {
        let normalized = validate_relative_file_path(path, field, entity)?;
        if !seen.insert(normalized.clone()) {
            return Err(format!(
                "{}的 {} 包含重复路径：{}",
                entity, field, normalized
            ));
        }
        result.push(normalized);
    }
    Ok(result)
}

pub(crate) fn validate_subtask(
    subtask: &project::Subtask,
    entity: &str,
) -> Result<Vec<String>, String> {
    let allowed = validate_path_list(
        &subtask.allowed_file_paths,
        "allowed_file_paths",
        entity,
        true,
    )?;
    let new_files = validate_path_list(&subtask.new_file_paths, "new_file_paths", entity, false)?;

    let mut authorized = BTreeSet::new();
    authorized.extend(allowed);
    authorized.extend(new_files);
    Ok(authorized.into_iter().collect())
}

pub(crate) fn validate_subtasks(subtasks: &[project::Subtask]) -> Result<(), String> {
    if subtasks.is_empty() {
        return Err("执行计划为空".to_string());
    }
    for (index, subtask) in subtasks.iter().enumerate() {
        validate_subtask(subtask, &format!("第 {} 个小阶段", index + 1))?;
    }
    Ok(())
}

pub(crate) fn validate_subtasks_in_project(
    subtasks: &[project::Subtask],
    project_path: &str,
) -> Result<(), String> {
    validate_subtasks(subtasks)?;
    let root = std::fs::canonicalize(project_path)
        .map_err(|error| format!("无法解析项目根目录 {}：{}", project_path, error))?;

    for (index, subtask) in subtasks.iter().enumerate() {
        let entity = format!("第 {} 个小阶段", index + 1);
        for relative in validate_subtask(subtask, &entity)? {
            let candidate = root.join(&relative);
            if candidate.is_dir() {
                return Err(format!("{}的文件范围指向目录：{}", entity, relative));
            }
            let mut anchor = candidate.as_path();
            while !anchor.exists() {
                anchor = anchor
                    .parent()
                    .ok_or_else(|| format!("{}的文件范围无法定位到项目内：{}", entity, relative))?;
            }
            let canonical_anchor = std::fs::canonicalize(anchor)
                .map_err(|error| format!("无法解析授权路径 {}：{}", relative, error))?;
            if !canonical_anchor.starts_with(&root) {
                return Err(format!(
                    "{}的文件范围通过符号链接越出项目：{}",
                    entity, relative
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn out_of_scope_changes(
    changed_files: &[String],
    authorized_paths: &[String],
) -> Vec<String> {
    let authorized: BTreeSet<&str> = authorized_paths.iter().map(String::as_str).collect();
    changed_files
        .iter()
        .filter(|path| !authorized.contains(path.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subtask(allowed: &[&str], new_files: &[&str]) -> project::Subtask {
        project::Subtask {
            id: "subtask-1".to_string(),
            title: "测试".to_string(),
            prompt: "测试".to_string(),
            status: project::SubtaskStatus::Pending,
            test_report: String::new(),
            execution_result: None,
            test_result: None,
            retry_count: 0,
            auto_tag: None,
            order: 1,
            goal: "测试".to_string(),
            allowed_file_paths: allowed.iter().map(|path| path.to_string()).collect(),
            new_file_paths: new_files.iter().map(|path| path.to_string()).collect(),
            evidence_files: vec![],
            context_summary: String::new(),
            acceptance_criteria: vec!["通过".to_string()],
            stop_rules: vec!["越界时停止".to_string()],
            execution_prompt: "测试".to_string(),
            confirmed_by_user: None,
            confirmed_at: None,
            confirmation_notes: None,
        }
    }

    #[test]
    fn validates_exact_project_relative_paths() {
        let valid = subtask(&["src/main.rs"], &["src/new.rs"]);
        assert_eq!(
            validate_subtask(&valid, "第 1 个小阶段").unwrap(),
            vec!["src/main.rs".to_string(), "src/new.rs".to_string()]
        );

        for invalid in [
            "",
            ".",
            "../outside",
            "/tmp/file",
            "C:/tmp/file",
            "src\\main.rs",
            "src/",
            "src/*.rs",
        ] {
            let task = subtask(&[invalid], &[]);
            assert!(validate_subtask(&task, "第 1 个小阶段").is_err());
        }
    }

    #[test]
    fn detects_changes_outside_the_contract() {
        let changed = vec!["src/main.rs".to_string(), "README.md".to_string()];
        let authorized = vec!["src/main.rs".to_string()];
        assert_eq!(
            out_of_scope_changes(&changed, &authorized),
            vec!["README.md".to_string()]
        );
    }
}
