use crate::project;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

const MAX_SNIPPET_CHARS: usize = 1_200;
const MAX_TOTAL_SNIPPET_CHARS: usize = 4_800;
const MAX_PLANNING_FILES: usize = 80;
const MAX_PLANNING_FACT_ITEMS: usize = 160;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "coverage",
    ".next",
    "__pycache__",
    ".venv",
];

fn sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn git_head(project_path: &str) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project_path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

fn quoted_values(content: &str, marker: &str) -> Vec<String> {
    let mut values = BTreeSet::new();
    for (offset, _) in content.match_indices(marker) {
        let tail = &content[offset + marker.len()..];
        if let Some(quote) = tail.chars().find(|ch| *ch == '\'' || *ch == '"') {
            let after = tail.split_once(quote).map(|(_, value)| value).unwrap_or("");
            if let Some((value, _)) = after.split_once(quote) {
                if !value.trim().is_empty() && value.len() <= 128 {
                    values.insert(value.to_string());
                }
            }
        }
    }
    values.into_iter().collect()
}

fn symbols(content: &str) -> Vec<String> {
    let mut result = BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        for prefix in [
            "fn ",
            "function ",
            "class ",
            "struct ",
            "enum ",
            "const ",
            "let ",
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let name = rest
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
                    .collect::<String>();
                if name.len() > 1 {
                    result.insert(name);
                }
            }
        }
    }
    result.into_iter().collect()
}

pub(crate) fn capture(
    project_path: &str,
    paths: &[String],
    accepted_deviations: Vec<String>,
) -> Result<project::ProjectFactSnapshot, String> {
    let root = std::fs::canonicalize(project_path)
        .map_err(|error| format!("无法解析项目事实根目录 {}：{}", project_path, error))?;
    let mut file_hashes = BTreeMap::new();
    let mut all_symbols = BTreeSet::new();
    let mut storage_keys = BTreeSet::new();
    let mut dom_ids = BTreeSet::new();
    let mut event_bindings = BTreeSet::new();
    let mut snippets = Vec::new();

    let mut snippet_chars = 0;
    for relative in paths.iter().collect::<BTreeSet<_>>() {
        let relative_path = Path::new(relative);
        if relative.trim().is_empty()
            || relative_path.is_absolute()
            || !relative_path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "项目事实文件必须是精确项目内相对路径：{}",
                relative
            ));
        }
        let full = root.join(relative_path);
        if !full.exists() {
            file_hashes.insert(relative.clone(), "missing".to_string());
            continue;
        }
        let canonical = std::fs::canonicalize(&full)
            .map_err(|error| format!("解析事实文件 {} 失败：{}", relative, error))?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(format!("项目事实文件越出项目或不是普通文件：{}", relative));
        }
        let bytes = std::fs::read(&full)
            .map_err(|error| format!("读取事实文件 {} 失败：{}", relative, error))?;
        file_hashes.insert(relative.clone(), sha256(&bytes));
        if let Ok(content) = String::from_utf8(bytes) {
            all_symbols.extend(symbols(&content));
            storage_keys.extend(quoted_values(&content, "localStorage."));
            dom_ids.extend(quoted_values(&content, "getElementById("));
            event_bindings.extend(quoted_values(&content, "addEventListener("));
            if snippet_chars < MAX_TOTAL_SNIPPET_CHARS {
                let remaining = MAX_TOTAL_SNIPPET_CHARS - snippet_chars;
                let snippet: String = content
                    .chars()
                    .take(MAX_SNIPPET_CHARS.min(remaining))
                    .collect();
                snippet_chars += snippet.chars().count();
                snippets.push(format!("{relative}:\n{snippet}"));
            }
        }
    }
    let fingerprint_input = serde_json::to_vec(&(
        &file_hashes,
        &all_symbols,
        &storage_keys,
        &dom_ids,
        &event_bindings,
        &accepted_deviations,
    ))
    .map_err(|error| format!("序列化项目事实失败：{}", error))?;
    Ok(project::ProjectFactSnapshot {
        git_head: git_head(project_path),
        file_hashes,
        symbols: all_symbols.into_iter().collect(),
        storage_keys: storage_keys.into_iter().collect(),
        dom_ids: dom_ids.into_iter().collect(),
        event_bindings: event_bindings.into_iter().collect(),
        relevant_snippets: snippets,
        accepted_deviations,
        structural_fingerprint: sha256(&fingerprint_input),
        captured_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn planning_file_rank(path: &str) -> u8 {
    let name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "package.json"
            | "cargo.toml"
            | "pyproject.toml"
            | "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "readme.md"
    ) {
        0
    } else {
        1
    }
}

fn is_planning_text_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "dockerfile" | "makefile" | "cargo.toml" | "package.json" | "go.mod"
    ) {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "html"
            | "css"
            | "vue"
            | "svelte"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "sql"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "xml"
            | "md"
    )
}

fn planning_paths(project_path: &str) -> Result<Vec<String>, String> {
    let root = Path::new(project_path);
    if !root.is_dir() {
        return Err(format!("项目事实扫描路径不可用：{}", project_path));
    }
    let mut paths = walkdir::WalkDir::new(root)
        .max_depth(8)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.path() == root
                || !entry.file_type().is_dir()
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIP_DIRS.contains(&name))
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_planning_text_file(entry.path()))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
        })
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| (planning_file_rank(path), path.clone()));
    paths.truncate(MAX_PLANNING_FILES);
    Ok(paths)
}

fn limited(items: &[String]) -> Vec<&str> {
    items
        .iter()
        .take(MAX_PLANNING_FACT_ITEMS)
        .map(String::as_str)
        .collect()
}

/// Compressed, current repository facts shared by plan generation and review.
/// Full source files are deliberately excluded from this context.
pub(crate) fn planning_context(project: &project::Project) -> Result<String, String> {
    let paths = planning_paths(&project.project_path)?;
    let facts = capture(&project.project_path, &paths, accepted_deviations(project))?;
    serde_json::to_string_pretty(&serde_json::json!({
        "git_head": facts.git_head,
        "structural_fingerprint": facts.structural_fingerprint,
        "scanned_files": paths,
        "symbols": limited(&facts.symbols),
        "storage_keys": limited(&facts.storage_keys),
        "dom_ids": limited(&facts.dom_ids),
        "event_bindings": limited(&facts.event_bindings),
        "relevant_snippets": facts.relevant_snippets,
        "accepted_deviations": facts.accepted_deviations,
    }))
    .map_err(|error| format!("序列化计划项目事实失败：{}", error))
}

pub(crate) fn has_drift(
    previous: Option<&project::ProjectFactSnapshot>,
    current: &project::ProjectFactSnapshot,
) -> bool {
    previous.is_some_and(|old| old.structural_fingerprint != current.structural_fingerprint)
}

pub(crate) fn next_task_needs_scan_or_calibration(
    project: &project::Project,
) -> Result<bool, String> {
    let task = project
        .milestones
        .iter()
        .find(|milestone| milestone.id == project.current_milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid| mid.id == project.current_mid_stage_id)
        })
        .and_then(|mid| {
            mid.subtasks
                .iter()
                .find(|task| task.status == project::SubtaskStatus::Pending)
        })
        .ok_or_else(|| "没有待扫描的小阶段。".to_string())?;
    let Some(previous) = task.fact_snapshot.as_ref() else {
        return Ok(true);
    };
    let current = capture(
        &project.project_path,
        &snapshot_paths(task),
        accepted_deviations(project),
    )?;
    Ok(has_drift(Some(previous), &current))
}

pub(crate) fn accepted_deviations(project: &project::Project) -> Vec<String> {
    project
        .milestones
        .iter()
        .flat_map(|milestone| &milestone.mid_stages)
        .flat_map(|mid_stage| &mid_stage.subtasks)
        .filter_map(|subtask| {
            subtask
                .human_verification
                .as_ref()
                .and_then(|verification| {
                    (verification.resolution == project::HumanResolution::AcceptDeviation).then(
                        || {
                            format!(
                                "{}：{}（验收项 {:?}）",
                                subtask.title,
                                verification.verification_reason,
                                verification.accepted_criteria,
                            )
                        },
                    )
                })
        })
        .collect()
}

pub(crate) fn snapshot_paths(subtask: &project::Subtask) -> Vec<String> {
    subtask
        .allowed_file_paths
        .iter()
        .chain(&subtask.new_file_paths)
        .chain(&subtask.evidence_files)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fact_drift() {
        let mut old = project::ProjectFactSnapshot::default();
        old.structural_fingerprint = "a".to_string();
        let mut current = old.clone();
        current.structural_fingerprint = "b".to_string();
        assert!(has_drift(Some(&old), &current));
        assert!(!has_drift(None, &current));
    }

    #[test]
    fn planning_context_extracts_current_facts_without_full_files() {
        let root =
            std::env::temp_dir().join(format!("metheus-planning-facts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("index.html"),
            "<div id=\"search\"></div><script>function boot() { localStorage.setItem('tabzero_bookmarks', '[]'); document.getElementById('search').addEventListener('click', boot); }</script>",
        )
        .unwrap();
        let mut project = project::Project::new("facts");
        project.project_path = root.to_string_lossy().to_string();

        let context = planning_context(&project).unwrap();
        assert!(context.contains("index.html"));
        assert!(context.contains("tabzero_bookmarks"));
        assert!(context.contains("search"));
        assert!(context.contains("click"));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fact_capture_rejects_paths_outside_the_project() {
        let root =
            std::env::temp_dir().join(format!("metheus-fact-boundary-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(capture(
            &root.to_string_lossy(),
            &["../outside.txt".to_string()],
            vec![],
        )
        .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
