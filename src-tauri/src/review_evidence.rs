use crate::project;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MAX_REVIEW_EVIDENCE_CHARS: usize = 30_000;
const MAX_REVIEW_COVERAGE_CHARS: usize = 6_000;
const MAX_REVIEW_BLOCK_CHARS: usize = MAX_REVIEW_EVIDENCE_CHARS - MAX_REVIEW_COVERAGE_CHARS - 1;
const FULL_FILE_PREVIEW_CHARS: usize = 4_000;
const MAX_CRITERION_EVIDENCE_CHARS: usize = 6_000;
pub(crate) const MAX_TARGETED_CRITERIA: usize = 8;
const EVIDENCE_CONTEXT_LINES: usize = 6;
const MAX_ANCHOR_TERMS: usize = 64;
const MAX_PREFERRED_FILES: usize = 32;

#[derive(Debug, Clone, Default)]
pub(crate) struct ReviewEvidenceRequest {
    pub strategy: project::ReviewEvidenceStrategy,
    pub target_criterion_indices: Vec<u32>,
    pub anchor_terms: Vec<String>,
    pub preferred_files: Vec<String>,
}

impl ReviewEvidenceRequest {
    pub(crate) fn for_task(
        task: &project::Subtask,
        strategy: project::ReviewEvidenceStrategy,
        target_criterion_indices: Vec<u32>,
    ) -> Self {
        let mut anchors = BTreeSet::new();
        let mut preferred_files = BTreeSet::new();
        extend_clean(&mut anchors, &task.required_identifiers);
        extend_clean(&mut anchors, &task.related_symbols);
        extend_clean(&mut anchors, &task.expected_artifacts);
        extend_clean(&mut preferred_files, &task.evidence_files);
        extend_clean(&mut preferred_files, &task.allowed_file_paths);
        extend_clean(&mut preferred_files, &task.new_file_paths);
        extend_clean(&mut preferred_files, &task.read_file_paths);
        extend_clean(&mut preferred_files, &task.write_file_paths);
        if let Some(contract) = &task.contract_snapshot {
            extend_clean(&mut anchors, &contract.artifacts.expected_identifiers);
            extend_clean(&mut anchors, &contract.artifacts.related_symbols);
            extend_clean(&mut anchors, &contract.artifacts.expected_artifacts);
            extend_clean(&mut preferred_files, &contract.artifacts.expected_files);
            extend_clean(&mut preferred_files, &contract.artifacts.read_file_paths);
            extend_clean(&mut preferred_files, &contract.artifacts.write_file_paths);
        }
        let selected = target_criterion_indices
            .iter()
            .filter_map(|index| {
                task.acceptance_criteria
                    .get(index.saturating_sub(1) as usize)
            })
            .cloned()
            .collect::<Vec<_>>();
        anchors.extend(crate::plan_contract::acceptance_identifiers(&selected));
        Self {
            strategy,
            target_criterion_indices,
            anchor_terms: anchors.into_iter().take(MAX_ANCHOR_TERMS).collect(),
            preferred_files: preferred_files
                .into_iter()
                .take(MAX_PREFERRED_FILES)
                .collect(),
        }
    }
}

fn extend_clean(target: &mut BTreeSet<String>, values: &[String]) {
    target.extend(
        values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    );
}

#[derive(Debug)]
pub(crate) struct ReviewEvidence {
    pub(crate) rendered: String,
    pub(crate) status: project::ReviewEvidenceStatus,
    pub(crate) summary: String,
    pub(crate) blocks: BTreeMap<String, project::ReviewEvidenceReference>,
}

pub(crate) fn merge_evidence_status(
    current: &mut project::ReviewEvidenceStatus,
    next: project::ReviewEvidenceStatus,
) {
    use project::ReviewEvidenceStatus::{Complete, Partial, Unavailable};
    if matches!(next, Unavailable) || matches!((&*current, next), (Complete, Partial)) {
        *current = next;
    }
}

pub(crate) fn truncate_head_tail(text: &str, limit: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= limit {
        return (text.to_string(), false);
    }
    if limit < 80 {
        return (text.chars().take(limit).collect(), true);
    }
    let marker_reserve = 60.min(limit / 3);
    let content_budget = limit.saturating_sub(marker_reserve);
    let head_budget = content_budget / 2;
    let tail_budget = content_budget.saturating_sub(head_budget);
    let head = text.chars().take(head_budget).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_budget)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let omitted = total.saturating_sub(head.chars().count() + tail.chars().count());
    (
        format!("{head}\n...[证据截断：省略 {omitted} 个字符]...\n{tail}"),
        true,
    )
}

fn number_lines(text: &str, starting_line: usize) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| format!("{:>6} | {}", starting_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_file_preview(content: &str) -> (String, bool) {
    let total = content.chars().count();
    if total <= FULL_FILE_PREVIEW_CHARS {
        return (number_lines(content, 1), false);
    }
    let head = content.chars().take(1_000).collect::<String>();
    let tail = content
        .chars()
        .rev()
        .take(3_000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let tail_start = content.lines().count().saturating_sub(tail.lines().count()) + 1;
    let omitted = total.saturating_sub(head.chars().count() + tail.chars().count());
    (
        format!(
            "{}\n...[文件内容省略 {omitted} 个字符；省略区域不能作为代码不存在的依据]...\n{}",
            number_lines(&head, 1),
            number_lines(&tail, tail_start),
        ),
        true,
    )
}

fn git_diff_for_file(project_path: &str, file: &str) -> Result<String, String> {
    let literal_pathspec = format!(":(literal){file}");
    let output = std::process::Command::new("git")
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=12",
            "HEAD",
            "--",
            &literal_pathspec,
        ])
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("运行 git diff 失败：{error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!(
            "git diff 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[derive(Debug, Clone)]
struct DiffHunk {
    start_line: u32,
    end_line: u32,
    body: String,
}

pub(crate) fn parse_diff_hunks(diff: &str) -> Vec<(u32, u32, String)> {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    for line in diff.lines() {
        if line.starts_with("@@ ") {
            if let Some(hunk) = current.take() {
                hunks.push((hunk.start_line, hunk.end_line, hunk.body));
            }
            let range = line
                .split_whitespace()
                .find(|part| part.starts_with('+'))
                .unwrap_or("+1,1")
                .trim_start_matches('+');
            let mut parts = range.split(',');
            let start: u32 = parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let count: u32 = parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            current = Some(DiffHunk {
                start_line: start,
                end_line: start.saturating_add(count.saturating_sub(1)),
                body: format!("{line}\n"),
            });
        } else if let Some(hunk) = current.as_mut() {
            hunk.body.push_str(line);
            hunk.body.push('\n');
        }
    }
    if let Some(hunk) = current {
        hunks.push((hunk.start_line, hunk.end_line, hunk.body));
    }
    hunks
}

pub(crate) fn render_diff_hunk(body: &str) -> String {
    body.trim_end().to_string()
}

pub(crate) fn criterion_indices(criteria: &[String], request: &ReviewEvidenceRequest) -> Vec<u32> {
    match request.strategy {
        project::ReviewEvidenceStrategy::Standard => (1..=criteria.len() as u32).collect(),
        project::ReviewEvidenceStrategy::Targeted
        | project::ReviewEvidenceStrategy::ExpandedTargeted => {
            let mut indices = request
                .target_criterion_indices
                .iter()
                .copied()
                .filter(|index| *index > 0 && (*index as usize) <= criteria.len())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            indices.truncate(MAX_TARGETED_CRITERIA);
            indices
        }
    }
}

fn criterion_terms(criterion: &str, anchors: &[String], expanded: bool) -> Vec<String> {
    let mut terms = crate::plan_contract::acceptance_identifiers(&[criterion.to_string()]);
    for token in criterion.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '`' | '\'' | '"' | '(' | ')' | '[' | ']' | ',' | '，' | '。' | '：' | ':'
            )
    }) {
        let token = token.trim_matches(|ch: char| matches!(ch, '#' | '.' | ';'));
        if token.chars().count() >= 3 {
            terms.insert(token.to_string());
        }
    }
    extend_clean(&mut terms, anchors);
    if expanded && needs_lifecycle_context(criterion, anchors) {
        terms.extend(
            [
                "DOMContentLoaded",
                "window.onload",
                "addEventListener",
                "localStorage",
                "sessionStorage",
                "getItem",
                "setItem",
                "initialize",
                "init",
                "load",
                "save",
                "render",
            ]
            .into_iter()
            .map(str::to_string),
        );
    }
    terms.into_iter().take(MAX_ANCHOR_TERMS).collect()
}

fn needs_lifecycle_context(criterion: &str, anchors: &[String]) -> bool {
    let text = std::iter::once(criterion)
        .chain(anchors.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    [
        "init",
        "load",
        "save",
        "render",
        "storage",
        "persist",
        "初始化",
        "加载",
        "恢复",
        "保存",
        "持久",
        "渲染",
    ]
    .iter()
    .any(|token| text.contains(token))
}

fn normalized_needle(term: &str) -> String {
    term.split('.')
        .next_back()
        .unwrap_or(term)
        .trim()
        .trim_end_matches("()")
        .trim_matches(|ch: char| {
            !ch.is_alphanumeric() && ch != '_' && ch != '-' && !ch.is_alphabetic()
        })
        .to_string()
}

fn is_definition(line: &str, needle: &str) -> bool {
    let compact = line.trim();
    [
        format!("function {needle}"),
        format!("fn {needle}"),
        format!("def {needle}"),
        format!("class {needle}"),
        format!("const {needle}"),
        format!("let {needle}"),
        format!("var {needle}"),
    ]
    .iter()
    .any(|pattern| compact.contains(pattern))
}

fn is_lifecycle_line(line: &str) -> bool {
    [
        "DOMContentLoaded",
        "window.onload",
        "addEventListener",
        "localStorage",
        "sessionStorage",
        "getItem",
        "setItem",
        "initialize",
        "init(",
        "load(",
        "save(",
        "render(",
    ]
    .iter()
    .any(|token| line.contains(token))
}

#[derive(Debug, Clone)]
struct EvidenceWindow {
    start: usize,
    end: usize,
    kind: project::EvidenceSourceKind,
}

fn evidence_windows(content: &str, terms: &[String], radius: usize) -> Vec<EvidenceWindow> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut windows = Vec::new();
    for term in terms {
        let needle = normalized_needle(term);
        if needle.chars().count() < 3 {
            continue;
        }
        for (index, line) in lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(&needle))
            .take(4)
        {
            let kind = if is_definition(line, &needle) {
                project::EvidenceSourceKind::SymbolDefinition
            } else if is_lifecycle_line(line) {
                project::EvidenceSourceKind::LifecycleContext
            } else {
                project::EvidenceSourceKind::SymbolReference
            };
            windows.push(EvidenceWindow {
                start: index.saturating_sub(radius),
                end: (index + radius + 1).min(lines.len()),
                kind,
            });
        }
    }
    windows.sort_by_key(|window| (window.start, window.end));
    let mut merged: Vec<EvidenceWindow> = Vec::new();
    for window in windows {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| window.start <= previous.end)
        {
            previous.end = previous.end.max(window.end);
            if source_priority(&window.kind) > source_priority(&previous.kind) {
                previous.kind = window.kind;
            }
        } else {
            merged.push(window);
        }
    }
    merged
}

fn source_priority(kind: &project::EvidenceSourceKind) -> u8 {
    match kind {
        project::EvidenceSourceKind::SymbolDefinition => 3,
        project::EvidenceSourceKind::LifecycleContext => 2,
        _ => 1,
    }
}

fn ordered_files(files: &[String], preferred: &[String]) -> Vec<String> {
    let allowed = files.iter().cloned().collect::<BTreeSet<_>>();
    let mut ordered = preferred
        .iter()
        .filter(|file| allowed.contains(*file))
        .cloned()
        .collect::<Vec<_>>();
    let already_ordered = ordered.iter().cloned().collect::<BTreeSet<_>>();
    ordered.extend(
        files
            .iter()
            .filter(|file| !already_ordered.contains(*file))
            .cloned(),
    );
    ordered
}

fn authorized_file_path(project_path: &str, file: &str) -> Result<std::path::PathBuf, String> {
    let root = std::fs::canonicalize(project_path)
        .map_err(|error| format!("无法解析项目根目录：{error}"))?;
    let candidate = root.join(file);
    let resolved =
        std::fs::canonicalize(&candidate).map_err(|error| format!("目标文件读取失败：{error}"))?;
    if !resolved.starts_with(&root) {
        return Err("授权文件通过符号链接越出项目根目录".to_string());
    }
    if !resolved.is_file() {
        return Err("授权路径不是文件".to_string());
    }
    Ok(resolved)
}

fn push_evidence_block(
    rendered: &mut String,
    blocks: &mut BTreeMap<String, project::ReviewEvidenceReference>,
    status: &mut project::ReviewEvidenceStatus,
    seen: &mut BTreeMap<String, String>,
    file: &str,
    source_kind: project::EvidenceSourceKind,
    start_line: Option<u32>,
    end_line: Option<u32>,
    body: &str,
) -> Option<String> {
    let dedupe_key = format!("{file}:{start_line:?}:{end_line:?}:{source_kind:?}");
    if let Some(existing) = seen.get(&dedupe_key) {
        return Some(existing.clone());
    }
    let remaining = MAX_REVIEW_BLOCK_CHARS.saturating_sub(rendered.chars().count());
    if remaining < 200 {
        merge_evidence_status(status, project::ReviewEvidenceStatus::Partial);
        return None;
    }
    let block_id = format!("E{:03}", blocks.len() + 1);
    let range = match (start_line, end_line) {
        (Some(start), Some(end)) => format!("lines {start}-{end}"),
        _ => "lines n/a".to_string(),
    };
    let header = format!("\n[{block_id} | {source_kind:?} | {file} | {range}]\n");
    let budget = remaining
        .saturating_sub(header.chars().count() + 1)
        .min(MAX_CRITERION_EVIDENCE_CHARS);
    let (body, truncated) = truncate_head_tail(body, budget);
    if truncated {
        merge_evidence_status(status, project::ReviewEvidenceStatus::Partial);
    }
    rendered.push_str(&header);
    rendered.push_str(&body);
    rendered.push('\n');
    blocks.insert(
        block_id.clone(),
        project::ReviewEvidenceReference {
            block_id: block_id.clone(),
            source_kind,
            file: file.to_string(),
            start_line,
            end_line,
        },
    );
    seen.insert(dedupe_key, block_id.clone());
    Some(block_id)
}

pub(crate) fn build_review_evidence_with_request(
    project_path: &str,
    files: &[String],
    criteria: &[String],
    request: &ReviewEvidenceRequest,
) -> ReviewEvidence {
    if files.is_empty() {
        return ReviewEvidence {
            rendered: "（没有可供审查的授权文件）".to_string(),
            status: project::ReviewEvidenceStatus::Unavailable,
            summary: "没有可供审查的授权文件".to_string(),
            blocks: BTreeMap::new(),
        };
    }
    let indices = criterion_indices(criteria, request);
    let ordered = ordered_files(files, &request.preferred_files);
    let mut rendered = String::new();
    let mut status = project::ReviewEvidenceStatus::Complete;
    let mut notes = Vec::new();
    let mut blocks = BTreeMap::new();
    let mut seen = BTreeMap::new();
    let mut coverage = BTreeMap::<u32, Vec<String>>::new();
    let expanded = request.strategy == project::ReviewEvidenceStrategy::ExpandedTargeted;

    for index in &indices {
        let criterion = criteria
            .get(*index as usize - 1)
            .map(String::as_str)
            .unwrap_or("");
        let terms = criterion_terms(criterion, &request.anchor_terms, expanded);
        for file in &ordered {
            let full_path = match authorized_file_path(project_path, file) {
                Ok(path) => path,
                Err(error) => {
                    merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
                    notes.push(format!("{file}: {error}"));
                    continue;
                }
            };
            let content = match std::fs::read_to_string(&full_path) {
                Ok(content) => content,
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
                    notes.push(format!("{file}: 二进制或非 UTF-8 文件"));
                    continue;
                }
                Err(error) => {
                    notes.push(format!("{file}: 目标文件读取失败：{error}"));
                    continue;
                }
            };
            let lines = content.lines().collect::<Vec<_>>();
            if expanded && needs_lifecycle_context(criterion, &request.anchor_terms) {
                if content.contains("setItem") && !content.contains("getItem") {
                    notes.push(format!("{file}: 找到存储写入但没有初始化读取入口"));
                }
                for term in &request.anchor_terms {
                    let needle = normalized_needle(term);
                    if needle.chars().count() < 3 {
                        continue;
                    }
                    let matches = lines
                        .iter()
                        .filter(|line| line.contains(&needle))
                        .collect::<Vec<_>>();
                    if !matches.is_empty()
                        && matches.iter().all(|line| is_definition(line, &needle))
                    {
                        notes.push(format!("{file}: 找到符号定义 `{needle}` 但没有调用位置"));
                    }
                }
            }
            for window in evidence_windows(
                &content,
                &terms,
                if expanded {
                    EVIDENCE_CONTEXT_LINES * 2
                } else {
                    EVIDENCE_CONTEXT_LINES
                },
            ) {
                let body = number_lines(
                    &lines[window.start..window.end].join("\n"),
                    window.start + 1,
                );
                if let Some(block_id) = push_evidence_block(
                    &mut rendered,
                    &mut blocks,
                    &mut status,
                    &mut seen,
                    file,
                    window.kind,
                    Some(window.start as u32 + 1),
                    Some(window.end as u32),
                    &body,
                ) {
                    coverage.entry(*index).or_default().push(block_id);
                }
            }
            match git_diff_for_file(project_path, file) {
                Ok(diff) => {
                    for (start, end, body) in parse_diff_hunks(&diff) {
                        let matches = request.strategy == project::ReviewEvidenceStrategy::Standard
                            || terms.iter().any(|term| {
                                let needle = normalized_needle(term);
                                needle.chars().count() >= 3 && body.contains(&needle)
                            });
                        if matches {
                            if let Some(block_id) = push_evidence_block(
                                &mut rendered,
                                &mut blocks,
                                &mut status,
                                &mut seen,
                                file,
                                project::EvidenceSourceKind::GitDiffHunk,
                                Some(start),
                                Some(end),
                                &render_diff_hunk(&body),
                            ) {
                                coverage.entry(*index).or_default().push(block_id);
                            }
                        }
                    }
                }
                Err(error) => notes.push(format!("{file}: {error}")),
            }
        }
    }

    if request.strategy == project::ReviewEvidenceStrategy::Standard {
        for file in &ordered {
            let Ok(full_path) = authorized_file_path(project_path, file) else {
                continue;
            };
            if let Ok(content) = std::fs::read_to_string(full_path) {
                let (preview, partial) = render_file_preview(&content);
                let _ = push_evidence_block(
                    &mut rendered,
                    &mut blocks,
                    &mut status,
                    &mut seen,
                    file,
                    project::EvidenceSourceKind::CurrentFileSnippet,
                    Some(1),
                    Some(content.lines().count().max(1) as u32),
                    &preview,
                );
                if partial {
                    merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
                    notes.push(format!(
                        "{file}: 当前文件仅提供头尾预览，省略区不代表代码不存在"
                    ));
                }
            }
        }
    } else {
        merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
    }
    for index in &indices {
        if coverage.get(index).is_none_or(Vec::is_empty) {
            merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
            notes.push(format!("验收项 {index}: 授权文件中没有命中目标锚点"));
        }
    }
    if blocks.is_empty() {
        status = project::ReviewEvidenceStatus::Unavailable;
    }
    let mut coverage_rendered = String::from("## 验收项证据覆盖\n");
    for index in &indices {
        coverage_rendered.push_str(&format!("criterion #{index}:\n"));
        if let Some(ids) = coverage.get(index) {
            let mut unique = ids.clone();
            unique.sort();
            unique.dedup();
            for id in unique {
                if let Some(reference) = blocks.get(&id) {
                    coverage_rendered.push_str(&format!(
                        "- {} {} lines {}-{} {:?}\n",
                        id,
                        reference.file,
                        reference.start_line.unwrap_or_default(),
                        reference.end_line.unwrap_or_default(),
                        reference.source_kind
                    ));
                }
            }
        } else {
            coverage_rendered.push_str("- no matching authorized evidence block\n");
        }
    }
    let (coverage_rendered, coverage_truncated) =
        truncate_head_tail(&coverage_rendered, MAX_REVIEW_COVERAGE_CHARS);
    if coverage_truncated {
        merge_evidence_status(&mut status, project::ReviewEvidenceStatus::Partial);
        notes.push("验收项证据覆盖清单已按预算截断".to_string());
    }
    rendered = format!("{coverage_rendered}\n{rendered}");
    debug_assert!(rendered.chars().count() <= MAX_REVIEW_EVIDENCE_CHARS);

    let status_label = match status {
        project::ReviewEvidenceStatus::Complete => "完整",
        project::ReviewEvidenceStatus::Partial => "部分",
        project::ReviewEvidenceStatus::Unavailable => "不可用",
    };
    let strategy_label = match request.strategy {
        project::ReviewEvidenceStrategy::Standard => "标准审查".to_string(),
        project::ReviewEvidenceStrategy::Targeted => format!("定向补证（验收项 {:?}）", indices),
        project::ReviewEvidenceStrategy::ExpandedTargeted => {
            format!("扩展定向补证（验收项 {:?}）", indices)
        }
    };
    let summary = if notes.is_empty() {
        format!(
            "{strategy_label}：证据{status_label}，覆盖 {} 个授权文件",
            ordered.len()
        )
    } else {
        format!("{strategy_label}：证据{status_label}：{}", notes.join("；"))
    };
    ReviewEvidence {
        rendered,
        status,
        summary,
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_middle_diff_hunks_with_new_line_ranges() {
        let diff = "diff --git a/a b/a\n@@ -1,2 +10,3 @@\n a\n+b\n@@ -20 +30,2 @@\n+c\n";
        let hunks = parse_diff_hunks(diff);
        assert_eq!((hunks[0].0, hunks[0].1), (10, 12));
        assert_eq!((hunks[1].0, hunks[1].1), (30, 31));
        assert!(hunks[1].2.contains("+c"));
    }

    #[test]
    fn task_request_includes_contract_anchors() {
        let mut task = project::Subtask::default();
        task.related_symbols = vec!["renderGroups".into()];
        let mut contract = crate::task_contract::compile_subtask(&task, None, 0);
        contract.artifacts.expected_identifiers = vec!["saveBookmarks".into()];
        contract.artifacts.expected_files = vec!["index.html".into()];
        task.contract_snapshot = Some(contract);
        let request = ReviewEvidenceRequest::for_task(
            &task,
            project::ReviewEvidenceStrategy::Targeted,
            vec![1],
        );
        assert!(request.anchor_terms.contains(&"renderGroups".to_string()));
        assert!(request.anchor_terms.contains(&"saveBookmarks".to_string()));
        assert!(request.preferred_files.contains(&"index.html".to_string()));
    }

    #[test]
    fn finds_definition_reference_and_storage_lifecycle_in_large_file() -> Result<(), String> {
        let path = std::env::temp_dir().join(format!("metheus-evidence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let content = format!(
            "{}\nfunction renderGroups() {{ localStorage.setItem('groups', '[]'); }}\n{}\ndocument.addEventListener('DOMContentLoaded', () => {{ localStorage.getItem('groups'); renderGroups(); }});\n{}",
            "const before = 1;\n".repeat(300),
            "const middle = 1;\n".repeat(50),
            "const after = 1;\n".repeat(300),
        );
        std::fs::write(path.join("index.html"), content).map_err(|error| error.to_string())?;
        let evidence = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".into()],
            &["页面加载时恢复并重新渲染分组".into()],
            &ReviewEvidenceRequest {
                strategy: project::ReviewEvidenceStrategy::ExpandedTargeted,
                target_criterion_indices: vec![1],
                anchor_terms: vec!["renderGroups".into()],
                preferred_files: vec!["index.html".into()],
            },
        );
        assert!(evidence.rendered.contains("renderGroups"));
        assert!(evidence.rendered.contains("setItem"));
        assert!(evidence.rendered.contains("getItem"));
        assert!(evidence.rendered.contains("DOMContentLoaded"));
        assert!(evidence
            .blocks
            .values()
            .all(|reference| reference.start_line.is_some()));
        assert!(evidence.blocks.values().any(|reference| {
            reference.source_kind == project::EvidenceSourceKind::SymbolDefinition
        }));
        assert!(evidence.blocks.values().any(|reference| {
            reference.source_kind == project::EvidenceSourceKind::LifecycleContext
        }));
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn bookmark_application_targeted_and_expanded_evidence_cover_middle_implementation(
    ) -> Result<(), String> {
        let path = std::env::temp_dir().join(format!("metheus-bookmarks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let content = format!(
            "<!doctype html>\n<script>\n{}\nfunction renderGroups() {{ return groups.map(group => group.name); }}\nfunction addGroup(name) {{ groups.push({{ name, bookmarks: [] }}); saveBookmarks(); renderGroups(); }}\nfunction deleteGroup(index) {{ groups.splice(index, 1); saveBookmarks(); renderGroups(); }}\nfunction addBookmark(group, bookmark) {{ group.bookmarks.push(bookmark); saveBookmarks(); }}\nfunction editBookmark(bookmark, title) {{ bookmark.title = title; saveBookmarks(); }}\nfunction deleteBookmark(group, index) {{ group.bookmarks.splice(index, 1); saveBookmarks(); }}\nfunction normalizeUrl(url) {{ return /^https?:/.test(url) ? url : `https://${{url}}`; }}\nfunction saveBookmarks() {{ localStorage.setItem('bookmarks', JSON.stringify(groups)); }}\n{}\ndocument.addEventListener('DOMContentLoaded', () => {{ groups = JSON.parse(localStorage.getItem('bookmarks') || '[]'); renderGroups(); }});\n{}\n</script>",
            "const fillerBefore = 1;\n".repeat(240),
            "const fillerMiddle = 1;\n".repeat(80),
            "const fillerAfter = 1;\n".repeat(240),
        );
        assert!(content.chars().count() > 4_000);
        std::fs::write(path.join("index.html"), content).map_err(|error| error.to_string())?;
        let criteria = vec![
            "分组渲染".into(),
            "分组增删".into(),
            "书签增删改".into(),
            "URL 前缀补全".into(),
            "保存到 localStorage".into(),
            "页面加载时恢复并重新渲染".into(),
        ];
        let task = project::Subtask {
            acceptance_criteria: criteria.clone(),
            related_symbols: vec![
                "renderGroups".into(),
                "addGroup".into(),
                "deleteGroup".into(),
                "addBookmark".into(),
                "editBookmark".into(),
                "deleteBookmark".into(),
                "normalizeUrl".into(),
                "saveBookmarks".into(),
            ],
            allowed_file_paths: vec!["index.html".into()],
            ..Default::default()
        };
        let targeted = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".into()],
            &criteria,
            &ReviewEvidenceRequest::for_task(
                &task,
                project::ReviewEvidenceStrategy::Targeted,
                vec![1, 2, 3, 4, 5, 6],
            ),
        );
        assert!(targeted.rendered.contains("function renderGroups"));
        assert!(targeted.rendered.contains("function saveBookmarks"));
        let expanded = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".into()],
            &criteria,
            &ReviewEvidenceRequest::for_task(
                &task,
                project::ReviewEvidenceStrategy::ExpandedTargeted,
                vec![5, 6],
            ),
        );
        assert!(expanded.rendered.contains("localStorage.setItem"));
        assert!(expanded.rendered.contains("localStorage.getItem"));
        assert!(expanded.rendered.contains("DOMContentLoaded"));
        assert!(expanded.rendered.contains("renderGroups();"));
        assert!(expanded.blocks.values().all(|reference| {
            reference.start_line.is_some_and(|line| line > 0)
                && reference
                    .end_line
                    .is_some_and(|line| line >= reference.start_line.unwrap())
        }));
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn expanded_storage_evidence_reports_missing_initial_read() -> Result<(), String> {
        let path = std::env::temp_dir().join(format!(
            "metheus-storage-write-only-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        std::fs::write(
            path.join("index.html"),
            "function saveData() { localStorage.setItem('data', '{}'); }",
        )
        .map_err(|error| error.to_string())?;
        let evidence = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".into()],
            &["保存并在页面加载时恢复数据".into()],
            &ReviewEvidenceRequest {
                strategy: project::ReviewEvidenceStrategy::ExpandedTargeted,
                target_criterion_indices: vec![1],
                anchor_terms: vec!["saveData".into()],
                preferred_files: vec!["index.html".into()],
            },
        );
        assert!(evidence.summary.contains("写入但没有初始化读取"));
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn authorized_symlink_cannot_read_outside_project_root() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let path =
            std::env::temp_dir().join(format!("metheus-evidence-symlink-{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("metheus-evidence-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        std::fs::write(&outside, "const leakedSecret = true;\n")
            .map_err(|error| error.to_string())?;
        symlink(&outside, path.join("authorized.ts")).map_err(|error| error.to_string())?;

        let evidence = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["authorized.ts".into()],
            &["leakedSecret exists".into()],
            &ReviewEvidenceRequest::default(),
        );
        assert_eq!(evidence.status, project::ReviewEvidenceStatus::Unavailable);
        assert!(!evidence.rendered.contains("leakedSecret = true"));
        assert!(evidence.summary.contains("越出项目根目录"));

        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        std::fs::remove_file(outside).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn every_registered_block_is_visible_within_the_total_budget() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-visible-evidence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let mut content = String::new();
        let mut anchors = Vec::new();
        for index in 0..64 {
            let symbol = format!("targetSymbol{index}");
            anchors.push(symbol.clone());
            content.push_str(&format!(
                "function {symbol}() {{ return '{symbol}'; }}\n{}\n",
                "const filler = 'abcdefghijklmnopqrstuvwxyz';\n".repeat(12)
            ));
        }
        std::fs::write(path.join("large.js"), content).map_err(|error| error.to_string())?;

        let evidence = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["large.js".into()],
            &["all target symbols exist".into()],
            &ReviewEvidenceRequest {
                strategy: project::ReviewEvidenceStrategy::Targeted,
                target_criterion_indices: vec![1],
                anchor_terms: anchors,
                preferred_files: vec!["large.js".into()],
            },
        );
        assert!(evidence.rendered.chars().count() <= MAX_REVIEW_EVIDENCE_CHARS);
        assert!(!evidence.blocks.is_empty());
        assert!(evidence
            .blocks
            .keys()
            .all(|block_id| evidence.rendered.contains(&format!("[{block_id} |"))));

        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }
}
