use crate::project;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

const MAX_SNIPPET_CHARS: usize = 1_200;
const MAX_TOTAL_SNIPPET_CHARS: usize = 4_800;
const MAX_IDENTIFIER_CONTEXT_CHARS: usize = 1_600;
const MAX_IDENTIFIER_CONTEXTS: usize = 8;
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

fn collect_identifier_contexts(
    content: &str,
    identifiers: &[String],
) -> BTreeMap<String, Vec<String>> {
    let lines = content.lines().collect::<Vec<_>>();
    identifiers
        .iter()
        .filter(|identifier| !identifier.trim().is_empty())
        .take(MAX_IDENTIFIER_CONTEXTS)
        .filter_map(|identifier| {
            let mut contexts = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                if line.contains(identifier) {
                    let start = index.saturating_sub(2);
                    let end = (index + 3).min(lines.len());
                    let context = lines[start..end].join("\n");
                    contexts.push(context.chars().take(MAX_IDENTIFIER_CONTEXT_CHARS).collect());
                    if contexts.len() >= 3 {
                        break;
                    }
                }
            }
            (!contexts.is_empty()).then(|| (identifier.clone(), contexts))
        })
        .collect()
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
    for prefix in ["function ", "class ", "const ", "let "] {
        for (offset, _) in content.match_indices(prefix) {
            let rest = &content[offset + prefix.len()..];
            let name = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
                .collect::<String>();
            if name.len() > 1 {
                result.insert(name);
            }
        }
    }
    result.into_iter().collect()
}

fn storage_values(content: &str) -> Vec<String> {
    let mut values = BTreeSet::new();
    for (offset, _) in content.match_indices("localStorage.") {
        let tail = &content[offset + "localStorage.".len()..];
        let value = tail
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
            .collect::<String>();
        if value.len() > 1 {
            values.insert(value);
        }
    }
    for marker in ["setItem(", "getItem(", "removeItem("] {
        values.extend(quoted_values(content, marker));
    }
    values.into_iter().collect()
}

pub(crate) fn capture(
    project_path: &str,
    paths: &[String],
    accepted_deviations: Vec<String>,
) -> Result<project::ProjectFactSnapshot, String> {
    capture_with_identifiers(project_path, paths, accepted_deviations, &[])
}

pub(crate) fn capture_with_identifiers(
    project_path: &str,
    paths: &[String],
    accepted_deviations: Vec<String>,
    required_identifiers: &[String],
) -> Result<project::ProjectFactSnapshot, String> {
    let root = std::fs::canonicalize(project_path)
        .map_err(|error| format!("无法解析项目事实根目录 {}：{}", project_path, error))?;
    let mut file_hashes = BTreeMap::new();
    let mut all_symbols = BTreeSet::new();
    let mut storage_keys = BTreeSet::new();
    let mut dom_ids = BTreeSet::new();
    let mut event_bindings = BTreeSet::new();
    let mut snippets = Vec::new();
    let mut identifier_contexts = BTreeMap::new();

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
            storage_keys.extend(storage_values(&content));
            dom_ids.extend(quoted_values(&content, "getElementById("));
            dom_ids.extend(quoted_values(&content, "querySelector("));
            dom_ids.extend(quoted_values(&content, "querySelectorAll("));
            dom_ids.extend(quoted_values(&content, "id="));
            event_bindings.extend(quoted_values(&content, "addEventListener("));
            for (identifier, contexts) in
                collect_identifier_contexts(&content, required_identifiers)
            {
                identifier_contexts.entry(identifier).or_insert(contexts);
            }
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
        &identifier_contexts,
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
        identifier_contexts,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PlanningConstraints {
    pub accepted_deviations: Vec<String>,
    pub unresolved_issues: Vec<String>,
}

fn accepted_deviation_fact(task: &project::Subtask) -> Option<String> {
    if let Some(verification) = task
        .human_verification
        .as_ref()
        .filter(|verification| verification.resolution == project::HumanResolution::AcceptDeviation)
    {
        let reason = verification.verification_reason.trim();
        let reason = if reason.is_empty() {
            "未记录接受原因"
        } else {
            reason
        };
        return Some(format!(
            "{}：{}（验收项 {:?}）",
            task.title, reason, verification.accepted_criteria,
        ));
    }
    if task.child_tasks.is_empty() && task.status == project::SubtaskStatus::AcceptedDeviation {
        let criteria = task
            .acceptance_ledger
            .iter()
            .filter(|item| item.status == project::AcceptanceStatus::AcceptedDeviation)
            .map(|item| item.criterion.trim())
            .filter(|criterion| !criterion.is_empty())
            .collect::<Vec<_>>();
        return Some(if criteria.is_empty() {
            format!("{}：旧项目记录为已接受偏差", task.title)
        } else {
            format!("{}：已接受偏差（{}）", task.title, criteria.join("；"))
        });
    }
    None
}

fn collect_task_constraints(
    roots: &[project::Subtask],
    accepted_deviations: &mut BTreeSet<String>,
    unresolved_issues: &mut BTreeSet<String>,
) {
    let mut pending = roots.iter().collect::<Vec<_>>();
    while let Some(task) = pending.pop() {
        if let Some(deviation) = accepted_deviation_fact(task) {
            accepted_deviations.insert(deviation);
        }
        if task.status == project::SubtaskStatus::Rejected {
            unresolved_issues.insert(format!("任务「{}」已驳回", task.title));
        }
        for item in &task.acceptance_ledger {
            let status = match item.status {
                project::AcceptanceStatus::Unsatisfied => "未满足",
                project::AcceptanceStatus::Contradictory => "证据矛盾",
                _ => continue,
            };
            unresolved_issues.insert(format!(
                "任务「{}」验收项「{}」{}",
                task.title, item.criterion, status
            ));
        }
        pending.extend(task.child_tasks.iter());
    }
}

fn planning_constraints_with_scope(
    project: &project::Project,
    milestone_scope: Option<&BTreeSet<String>>,
) -> PlanningConstraints {
    let mut accepted_deviations = BTreeSet::new();
    let mut unresolved_issues = BTreeSet::new();
    for milestone in &project.milestones {
        if milestone_scope.is_some_and(|scope| !scope.contains(&milestone.id)) {
            continue;
        }
        collect_task_constraints(
            &milestone.subtasks,
            &mut accepted_deviations,
            &mut unresolved_issues,
        );
        for mid_stage in &milestone.mid_stages {
            collect_task_constraints(
                &mid_stage.subtasks,
                &mut accepted_deviations,
                &mut unresolved_issues,
            );
        }
    }
    PlanningConstraints {
        accepted_deviations: accepted_deviations
            .into_iter()
            .take(MAX_PLANNING_FACT_ITEMS)
            .collect(),
        unresolved_issues: unresolved_issues
            .into_iter()
            .take(MAX_PLANNING_FACT_ITEMS)
            .collect(),
    }
}

pub(crate) fn planning_constraints_for_milestones(
    project: &project::Project,
    milestone_ids: &BTreeSet<String>,
) -> PlanningConstraints {
    planning_constraints_with_scope(project, Some(milestone_ids))
}

fn planning_context_with_scope(
    project: &project::Project,
    milestone_scope: Option<&BTreeSet<String>>,
) -> Result<String, String> {
    let paths = planning_paths(&project.project_path)?;
    let constraints = planning_constraints_with_scope(project, milestone_scope);
    let facts = capture(
        &project.project_path,
        &paths,
        constraints.accepted_deviations,
    )?;
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
        "unresolved_issues": constraints.unresolved_issues,
    }))
    .map_err(|error| format!("序列化计划项目事实失败：{}", error))
}

/// Compressed, current repository facts shared by plan generation and review.
/// Full source files are deliberately excluded from this context.
pub(crate) fn planning_context(project: &project::Project) -> Result<String, String> {
    planning_context_with_scope(project, None)
}

pub(crate) fn planning_context_for_milestones(
    project: &project::Project,
    milestone_ids: &BTreeSet<String>,
) -> Result<String, String> {
    planning_context_with_scope(project, Some(milestone_ids))
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
    let scope = crate::plan_scope::PlanScope::resolve(project)?;
    let task = scope
        .subtasks(project)
        .iter()
        .find(|task| task.status == project::SubtaskStatus::Pending)
        .ok_or_else(|| "没有待扫描的小阶段。".to_string())?;
    let Some(previous) = task.fact_snapshot.as_ref() else {
        return Ok(true);
    };
    let current = capture_with_identifiers(
        &project.project_path,
        &snapshot_paths(task),
        accepted_deviations(project),
        &task.required_identifiers,
    )?;
    Ok(has_drift(Some(previous), &current))
}

pub(crate) fn accepted_deviations(project: &project::Project) -> Vec<String> {
    planning_constraints_with_scope(project, None).accepted_deviations
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

    fn test_task(id: &str, title: &str) -> project::Subtask {
        project::Subtask {
            id: id.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    fn test_mid_stage(tasks: Vec<project::Subtask>) -> project::MidStage {
        project::MidStage {
            id: "mid-1".to_string(),
            title: "测试中阶段".to_string(),
            version: "v0.1.1".to_string(),
            order: Some(1),
            status: project::MidStageStatus::Completed,
            subtasks: tasks,
            domain: None,
            test_log: None,
            created_at: String::new(),
            description: String::new(),
            tech_focus: String::new(),
            test_report: String::new(),
            completed_at: None,
            approved_at: None,
            git_tag: String::new(),
            plan_check_result: None,
            plan_approved_at: None,
            plan_revision: 0,
            plan_draft_revision: 0,
            plan_generated_at: None,
            plan_regeneration_count: 0,
            last_plan_failure_fingerprint: String::new(),
            last_plan_issue_count: 0,
            plan_no_progress_count: 0,
        }
    }

    fn test_milestone(id: &str, title: &str) -> project::Milestone {
        project::Milestone {
            id: id.to_string(),
            version: "v0.1".to_string(),
            title: title.to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: project::MilestoneStatus::Completed,
            mode: project::StageMode::Professional,
            mid_stages: vec![],
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: vec![],
            expected_output: String::new(),
            acceptance_criteria: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn planning_constraints_recursively_scope_dynamic_task_facts() {
        let mut quick_deviation = test_task("quick-deviation", "快速模式偏差");
        quick_deviation.human_verification = Some(project::HumanVerification {
            verification_kind: project::VerificationKind::HumanOverride,
            verification_reason: "保留兼容行为".to_string(),
            verified_at: String::new(),
            original_test_failure: String::new(),
            resolution: project::HumanResolution::AcceptDeviation,
            accepted_criteria: vec![1],
            dependency_check: String::new(),
            action_source: String::new(),
            execution_result_fingerprint: String::new(),
            task_tree_revision: 0,
            project_revision: 0,
        });

        let mut deep_deviation = test_task("deep-deviation", "动态叶子偏差");
        deep_deviation.human_verification = Some(project::HumanVerification {
            verification_kind: project::VerificationKind::HumanOverride,
            verification_reason: "接受性能债务".to_string(),
            verified_at: String::new(),
            original_test_failure: String::new(),
            resolution: project::HumanResolution::AcceptDeviation,
            accepted_criteria: vec![2],
            dependency_check: String::new(),
            action_source: String::new(),
            execution_result_fingerprint: String::new(),
            task_tree_revision: 0,
            project_revision: 0,
        });
        let mut rejected = test_task("rejected", "被驳回任务");
        rejected.status = project::SubtaskStatus::Rejected;
        let mut contradictory = test_task("contradictory", "矛盾任务");
        contradictory.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "行为必须稳定".to_string(),
            status: project::AcceptanceStatus::Contradictory,
            ..Default::default()
        }];
        let mut parent = test_task("parent", "动态父任务");
        parent.child_tasks = vec![deep_deviation, rejected, contradictory];

        let mut retained = test_milestone("retained", "已完成阶段");
        retained.subtasks = vec![quick_deviation];
        retained.mid_stages = vec![test_mid_stage(vec![parent])];

        let mut future_legacy = test_task("future-legacy", "未来旧偏差");
        future_legacy.status = project::SubtaskStatus::AcceptedDeviation;
        let mut future = test_milestone("future", "未来阶段");
        future.status = project::MilestoneStatus::Pending;
        future.subtasks = vec![future_legacy];

        let mut project = project::Project::new("recursive-facts");
        project.milestones = vec![retained, future];
        let scope = BTreeSet::from(["retained".to_string()]);
        let constraints = planning_constraints_for_milestones(&project, &scope);

        assert_eq!(constraints.accepted_deviations.len(), 2);
        assert!(constraints
            .accepted_deviations
            .iter()
            .any(|item| item.contains("快速模式偏差")));
        assert!(constraints
            .accepted_deviations
            .iter()
            .any(|item| item.contains("动态叶子偏差")));
        assert!(constraints
            .accepted_deviations
            .iter()
            .all(|item| !item.contains("未来旧偏差")));
        assert!(constraints
            .unresolved_issues
            .iter()
            .any(|item| item.contains("被驳回任务")));
        assert!(constraints
            .unresolved_issues
            .iter()
            .any(|item| item.contains("证据矛盾")));
        assert!(accepted_deviations(&project)
            .iter()
            .any(|item| item.contains("未来旧偏差")));
    }

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

    #[test]
    fn captures_identifier_context_from_file_tail_and_modern_dom_apis() {
        let root =
            std::env::temp_dir().join(format!("metheus-fact-context-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let content = format!(
            "{}\n<script>function tailAction() {{ localStorage.tail_key = 'x'; document.querySelector('#tail').addEventListener('click', tailAction); }}</script>",
            "\n".repeat(400)
        );
        std::fs::write(root.join("index.html"), content).unwrap();
        let facts = capture_with_identifiers(
            &root.to_string_lossy(),
            &["index.html".to_string()],
            vec![],
            &["tailAction".to_string()],
        )
        .unwrap();
        assert!(facts.identifier_contexts.contains_key("tailAction"));
        assert!(facts.symbols.contains(&"tailAction".to_string()));
        assert!(facts.storage_keys.contains(&"tail_key".to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
