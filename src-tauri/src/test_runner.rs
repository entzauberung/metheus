use crate::project;
use crate::review_evidence::{
    build_review_evidence_with_request, criterion_indices, ReviewEvidence, ReviewEvidenceRequest,
};
use crate::review_protocol::ModelReviewResponse;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

pub(crate) type FileSnapshot = BTreeMap<String, u64>;
pub(crate) type VerificationProgressReporter =
    Arc<dyn Fn(project::VerificationStage) + Send + Sync>;

fn report_verification_progress(
    reporter: Option<&VerificationProgressReporter>,
    stage: project::VerificationStage,
) {
    if let Some(reporter) = reporter {
        reporter(stage);
    }
}

fn display_path(path: &str, project_path: &str) -> String {
    std::path::Path::new(path)
        .strip_prefix(project_path)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Compare content fingerprints so additions, modifications, and deletions are all visible.
pub(crate) fn detect_changes(
    before: &FileSnapshot,
    after: &FileSnapshot,
    project_path: &str,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .map(|path| display_path(path, project_path))
        .collect()
}

#[cfg(test)]
mod change_detection_tests {
    use super::{
        authorized_review_files, detect_changes, git_changed_files, normalize_model_review,
        FileSnapshot,
    };
    use crate::automated_validation::AutomatedTestEvidence;
    use crate::project::ReviewEvidenceStatus;
    use crate::review_evidence::{
        build_review_evidence_with_request, truncate_head_tail, ReviewEvidence,
        ReviewEvidenceRequest,
    };
    use crate::review_protocol::{ModelCriterionReview, ModelReviewIssue, ModelReviewResponse};
    use std::process::Command;

    #[test]
    fn detects_added_modified_and_deleted_files() {
        let before = FileSnapshot::from([
            ("/project/deleted.rs".to_string(), 1),
            ("/project/modified.rs".to_string(), 2),
            ("/project/unchanged.rs".to_string(), 3),
        ]);
        let after = FileSnapshot::from([
            ("/project/added.rs".to_string(), 4),
            ("/project/modified.rs".to_string(), 5),
            ("/project/unchanged.rs".to_string(), 3),
        ]);

        assert_eq!(
            detect_changes(&before, &after, "/project"),
            vec!["added.rs", "deleted.rs", "modified.rs"]
        );
    }

    #[test]
    fn review_retry_reuses_automated_test_facts() {
        let previous = crate::project::TestResult {
            test_command: "cargo test --lib".to_string(),
            test_exit_code: Some(0),
            test_output_summary: "12 passed".to_string(),
            automated_test_status: crate::project::AutomatedTestStatus::Passed,
            ..Default::default()
        };

        let reused = AutomatedTestEvidence::from_previous(&previous);
        assert_eq!(reused.command, previous.test_command);
        assert_eq!(reused.exit_code, previous.test_exit_code);
        assert_eq!(reused.output_summary, previous.test_output_summary);
        assert_eq!(reused.status, previous.automated_test_status);
        assert!(reused
            .rendered
            .as_deref()
            .is_some_and(|value| value.contains("12 passed")));
    }

    #[test]
    fn git_changed_files_includes_tracked_and_untracked_evidence() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-test-evidence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| format!("创建测试目录失败：{}", error))?;
        let git = |args: &[&str]| -> Result<(), String> {
            let output = Command::new("git")
                .args(args)
                .current_dir(&path)
                .output()
                .map_err(|error| format!("运行 git 失败：{}", error))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        };
        git(&["init", "--quiet"])?;
        git(&["config", "user.name", "Metheus Test"])?;
        git(&["config", "user.email", "metheus-test@example.invalid"])?;
        std::fs::write(path.join("tracked.rs"), "fn before() {}\n")
            .map_err(|error| error.to_string())?;
        git(&["add", "tracked.rs"])?;
        git(&["commit", "--quiet", "-m", "baseline"])?;
        std::fs::write(path.join("tracked.rs"), "fn after() {}\n")
            .map_err(|error| error.to_string())?;
        std::fs::write(path.join("new.rs"), "fn new_file() {}\n")
            .map_err(|error| error.to_string())?;

        let project_path = path.to_string_lossy().to_string();
        assert_eq!(
            git_changed_files(&project_path),
            vec!["new.rs".to_string(), "tracked.rs".to_string()]
        );
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn authorized_review_scope_includes_unchanged_context_and_excludes_other_changes() {
        let selected = authorized_review_files(
            vec!["src/changed.ts".into(), "README.md".into()],
            &["src/changed.ts".into(), "src/context.ts".into()],
        );
        assert_eq!(
            selected,
            vec!["src/changed.ts".to_string(), "src/context.ts".to_string()]
        );
    }

    #[test]
    fn long_html_evidence_keeps_script_changes_at_file_tail() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-review-html-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let git = |args: &[&str]| -> Result<(), String> {
            let output = Command::new("git")
                .args(args)
                .current_dir(&path)
                .output()
                .map_err(|error| error.to_string())?;
            if output.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        };
        git(&["init", "--quiet"])?;
        git(&["config", "user.name", "Metheus Test"])?;
        git(&["config", "user.email", "metheus-test@example.invalid"])?;
        let baseline = format!(
            "<html>\n<body>\n{}</body>\n</html>\n",
            "<div>line</div>\n".repeat(300)
        );
        std::fs::write(path.join("index.html"), &baseline).map_err(|error| error.to_string())?;
        git(&["add", "index.html"])?;
        git(&["commit", "--quiet", "-m", "baseline"])?;
        let updated = baseline.replace(
            "</body>",
            "<script>\nfunction toggleTheme() { document.body.classList.toggle('dark'); }\n</script>\n</body>",
        );
        std::fs::write(path.join("index.html"), updated).map_err(|error| error.to_string())?;

        let evidence = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".to_string()],
            &["toggleTheme()".to_string()],
            &ReviewEvidenceRequest::default(),
        );
        assert!(evidence.rendered.contains("function toggleTheme"));
        assert!(evidence.rendered.contains("GitDiff"));
        assert_eq!(evidence.status, ReviewEvidenceStatus::Partial);
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn long_file_evidence_includes_identifier_context_from_middle() -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("metheus-review-context-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let content = format!(
            "{}\nfunction targetHandler(event) {{ event.preventDefault(); }}\n{}",
            "const filler = 1;\n".repeat(500),
            "const tail = 1;\n".repeat(500)
        );
        std::fs::write(path.join("index.html"), content).map_err(|error| error.to_string())?;
        let evidence = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".to_string()],
            &["event.preventDefault".to_string()],
            &ReviewEvidenceRequest::default(),
        );
        assert!(evidence.rendered.contains("SymbolReference"));
        assert!(evidence.rendered.contains("targetHandler"));
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn evidence_truncation_respects_unicode_boundaries_and_marks_omission() {
        let input = "审".repeat(1_000);
        let (rendered, truncated) = truncate_head_tail(&input, 160);
        assert!(truncated);
        assert!(rendered.contains("证据截断"));
        assert!(rendered.chars().count() <= 160);
    }

    #[test]
    fn evidence_builder_targeted_strategies_use_distinct_context() -> Result<(), String> {
        let path = std::env::temp_dir().join(format!(
            "metheus-targeted-evidence-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let content = format!(
            "{}\nfunction targetHandler(event) {{ localStorage.setItem('theme', 'dark'); }}\n{}",
            "const before = 1;\n".repeat(40),
            "const after = 1;\n".repeat(40)
        );
        std::fs::write(path.join("index.html"), content).map_err(|error| error.to_string())?;
        let criteria = vec!["`targetHandler()` 写入 localStorage theme".to_string()];
        let targeted = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".to_string()],
            &criteria,
            &ReviewEvidenceRequest {
                strategy: crate::project::ReviewEvidenceStrategy::Targeted,
                target_criterion_indices: vec![1],
                ..Default::default()
            },
        );
        let expanded = build_review_evidence_with_request(
            &path.to_string_lossy(),
            &["index.html".to_string()],
            &criteria,
            &ReviewEvidenceRequest {
                strategy: crate::project::ReviewEvidenceStrategy::ExpandedTargeted,
                target_criterion_indices: vec![1],
                ..Default::default()
            },
        );
        assert!(targeted.rendered.contains("targetHandler"));
        assert!(expanded.rendered.contains("targetHandler"));
        assert_ne!(targeted.rendered, expanded.rendered);
        assert!(
            targeted.rendered.chars().count() <= crate::review_evidence::MAX_REVIEW_EVIDENCE_CHARS
        );
        assert!(
            expanded.rendered.chars().count() <= crate::review_evidence::MAX_REVIEW_EVIDENCE_CHARS
        );
        std::fs::remove_dir_all(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn evidence_protocol_warning_does_not_block_satisfied_criterion() {
        let reference = crate::project::ReviewEvidenceReference {
            block_id: "E001".to_string(),
            source_kind: crate::project::EvidenceSourceKind::CurrentFileSnippet,
            file: "index.html".to_string(),
            start_line: Some(1),
            end_line: Some(3),
        };
        let evidence = ReviewEvidence {
            rendered: String::new(),
            status: crate::project::ReviewEvidenceStatus::Partial,
            summary: "文件部分展开".to_string(),
            blocks: std::collections::BTreeMap::from([("E001".to_string(), reference)]),
        };
        let response = ModelReviewResponse {
            passed: false,
            criterion_reviews: Some(vec![ModelCriterionReview {
                criterion_index: 1,
                conclusion: crate::project::CriterionReviewConclusion::Satisfied,
                confidence: 0.9,
                evidence_block_ids: vec!["E001".to_string()],
            }]),
            review_issues: vec![ModelReviewIssue {
                criterion_index: Some(1),
                criterion: "按钮可点击".to_string(),
                file: "index.html".to_string(),
                actual: "可以改用 let".to_string(),
                confidence: 0.9,
                severity: Some(crate::project::ReviewIssueSeverity::Suggestion),
                ..Default::default()
            }],
            ..Default::default()
        };
        let result = normalize_model_review(
            response,
            &["按钮可点击".to_string()],
            &["index.html".to_string()],
            &ReviewEvidenceRequest::default(),
            &evidence,
        );
        assert!(result.review_passed);
        assert!(result.passed);
        assert_eq!(result.review_issues.len(), 1);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("总体结论")));
    }

    #[test]
    fn review_issues_require_bound_criterion_file_and_evidence() {
        let reference = crate::project::ReviewEvidenceReference {
            block_id: "E001".to_string(),
            source_kind: crate::project::EvidenceSourceKind::CurrentFileSnippet,
            file: "index.html".to_string(),
            start_line: Some(1),
            end_line: Some(3),
        };
        let evidence = ReviewEvidence {
            rendered: String::new(),
            status: crate::project::ReviewEvidenceStatus::Complete,
            summary: String::new(),
            blocks: std::collections::BTreeMap::from([("E001".to_string(), reference)]),
        };
        let issue = |file: &str, evidence_block_ids: Vec<String>| ModelReviewIssue {
            criterion_index: Some(1),
            criterion: "按钮可点击".to_string(),
            file: file.to_string(),
            actual: "无法点击".to_string(),
            confidence: 0.9,
            severity: Some(crate::project::ReviewIssueSeverity::Blocking),
            evidence_block_ids,
            ..Default::default()
        };
        let result = normalize_model_review(
            ModelReviewResponse {
                criterion_reviews: Some(vec![ModelCriterionReview {
                    criterion_index: 1,
                    conclusion: crate::project::CriterionReviewConclusion::Unsatisfied,
                    confidence: 0.9,
                    evidence_block_ids: vec!["E001".to_string()],
                }]),
                review_issues: vec![
                    issue("index.html", vec!["E001".to_string()]),
                    issue("other.html", vec!["E001".to_string()]),
                    issue("index.html", vec![]),
                ],
                ..Default::default()
            },
            &["按钮可点击".to_string()],
            &["index.html".to_string()],
            &ReviewEvidenceRequest::default(),
            &evidence,
        );
        assert_eq!(result.review_issues.len(), 1);
        assert_eq!(result.review_issues[0].criterion_index, Some(1));
        assert_eq!(result.review_issues[0].criterion, "按钮可点击");
        assert_eq!(result.review_issues[0].file, "index.html");
        assert_eq!(result.review_issues[0].evidence_references.len(), 1);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("文件与证据块不一致")));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("criterion/file/evidence")));
    }

    #[test]
    fn review_protocol_removes_invalid_ids_without_relaxing_authorized_evidence() {
        let reference = crate::project::ReviewEvidenceReference {
            block_id: "E001".to_string(),
            source_kind: crate::project::EvidenceSourceKind::CurrentFileSnippet,
            file: "index.html".to_string(),
            start_line: Some(1),
            end_line: Some(3),
        };
        let evidence = ReviewEvidence {
            rendered: String::new(),
            status: crate::project::ReviewEvidenceStatus::Complete,
            summary: String::new(),
            blocks: std::collections::BTreeMap::from([("E001".to_string(), reference)]),
        };
        let response = ModelReviewResponse {
            passed: true,
            criterion_reviews: Some(vec![ModelCriterionReview {
                criterion_index: 1,
                conclusion: crate::project::CriterionReviewConclusion::Satisfied,
                confidence: 0.9,
                evidence_block_ids: vec!["UNKNOWN".to_string(), "E001".to_string()],
            }]),
            ..Default::default()
        };
        let result = normalize_model_review(
            response,
            &["按钮可点击".to_string()],
            &["index.html".to_string()],
            &ReviewEvidenceRequest::default(),
            &evidence,
        );
        assert!(result.review_passed);
        assert_eq!(result.criterion_reviews[0].evidence_references.len(), 1);
        assert_eq!(
            result.criterion_reviews[0].evidence_references[0].block_id,
            "E001"
        );
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("不存在或未授权")));
    }
}
/// 模拟一个"测试工程师"角色：
/// 自动检查当前项目里所有改动的代码，判断是否达到了子任务的目标，并返回测试结果（通过/问题/建议）
/// 扫描项目目录，递归返回所有文件路径列表（跳过 .git / node_modules / target）
pub(crate) fn get_tracked_files(project_path: &str) -> Vec<String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(project_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path().to_string_lossy().to_string();
        // 跳过 .git / node_modules / target 目录及其内部
        if path.contains("/.git/")
            || path.contains("/node_modules/")
            || path.contains("/target/")
            || path.ends_with("/.git")
            || path.ends_with("/node_modules")
            || path.ends_with("/target")
        {
            continue;
        }
        // 只记录文件
        if entry.file_type().is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
}

pub(crate) fn get_file_snapshot(project_path: &str) -> FileSnapshot {
    get_tracked_files(project_path)
        .into_iter()
        .map(|path| {
            let mut hasher = DefaultHasher::new();
            match std::fs::read(&path) {
                Ok(content) => content.hash(&mut hasher),
                Err(error) => error.kind().hash(&mut hasher),
            }
            (path, hasher.finish())
        })
        .collect()
}

fn git_changed_files(project_path: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for args in [
        vec!["diff", "--name-only", "-z", "HEAD"],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        if let Ok(output) = std::process::Command::new("git")
            .args(args)
            .current_dir(project_path)
            .output()
        {
            if output.status.success() {
                files.extend(
                    output
                        .stdout
                        .split(|byte| *byte == 0)
                        .filter(|path| !path.is_empty())
                        .map(|path| String::from_utf8_lossy(path).to_string()),
                );
            }
        }
    }
    files.into_iter().collect()
}

fn authorized_review_files(changed_files: Vec<String>, authorized_paths: &[String]) -> Vec<String> {
    let authorized = authorized_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut selected = changed_files
        .into_iter()
        .filter(|file| authorized.contains(file))
        .collect::<Vec<_>>();
    let mut seen = selected.iter().cloned().collect::<BTreeSet<_>>();
    selected.extend(
        authorized_paths
            .iter()
            .filter(|file| seen.insert((*file).clone()))
            .cloned(),
    );
    selected
}

fn resolve_evidence_references(
    block_ids: &[String],
    evidence: &ReviewEvidence,
    authorized_paths: &[String],
) -> Option<Vec<project::ReviewEvidenceReference>> {
    if block_ids.is_empty() {
        return None;
    }
    let authorized = authorized_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut references = Vec::new();
    let mut seen = BTreeSet::new();
    for block_id in block_ids {
        if !seen.insert(block_id) {
            continue;
        }
        let Some(reference) = evidence.blocks.get(block_id) else {
            continue;
        };
        if !authorized.is_empty() && !authorized.contains(reference.file.as_str()) {
            continue;
        }
        references.push(reference.clone());
    }
    (!references.is_empty()).then_some(references)
}

fn normalize_model_review(
    response: ModelReviewResponse,
    criteria: &[String],
    authorized_paths: &[String],
    request: &ReviewEvidenceRequest,
    evidence: &ReviewEvidence,
) -> project::TestResult {
    let requested_indices = criterion_indices(criteria, request);
    let mut warnings = response.warnings;
    let model_passed = response.passed;
    let mut review_issues = Vec::new();

    for issue in response.review_issues {
        let references =
            resolve_evidence_references(&issue.evidence_block_ids, evidence, authorized_paths);
        let requested_reference_count = issue
            .evidence_block_ids
            .iter()
            .collect::<BTreeSet<_>>()
            .len();
        if requested_reference_count > references.as_ref().map(Vec::len).unwrap_or_default() {
            warnings.push(format!(
                "审查问题引用了不存在或未授权的证据块：{}",
                issue.actual
            ));
        }
        let Some(references) = references else {
            warnings.push(format!(
                "审查问题缺少可复核的 criterion/file/evidence 绑定，已降为诊断：{}",
                issue.actual
            ));
            continue;
        };
        let criterion_index = issue
            .criterion_index
            .filter(|index| *index > 0 && (*index as usize) <= criteria.len())
            .or_else(|| {
                let criterion = issue.criterion.trim();
                (!criterion.is_empty())
                    .then(|| {
                        criteria
                            .iter()
                            .position(|candidate| candidate.trim() == criterion)
                    })
                    .flatten()
                    .map(|index| index as u32 + 1)
            });
        let Some(criterion_index) = criterion_index else {
            warnings.push(format!(
                "审查问题缺少有效的验收项绑定，已降为诊断：{}",
                issue.actual
            ));
            continue;
        };
        let evidence_files = references
            .iter()
            .map(|reference| reference.file.as_str())
            .collect::<BTreeSet<_>>();
        let file = if issue.file.trim().is_empty() {
            if evidence_files.len() == 1 {
                evidence_files
                    .iter()
                    .next()
                    .map(|file| (*file).to_string())
                    .unwrap_or_default()
            } else {
                warnings.push(format!(
                    "审查问题缺少唯一文件绑定，已降为诊断：{}",
                    issue.actual
                ));
                continue;
            }
        } else if evidence_files.contains(issue.file.trim()) {
            issue.file.trim().to_string()
        } else {
            warnings.push(format!(
                "审查问题的文件与证据块不一致，已降为诊断：{}",
                issue.actual
            ));
            continue;
        };
        let blocking = issue.severity == Some(project::ReviewIssueSeverity::Blocking);
        if issue.severity.is_none() || issue.confidence < 0.7 {
            warnings.push(format!(
                "审查问题未通过结构化校验，已降为诊断：{}",
                issue.actual
            ));
            continue;
        }
        review_issues.push(project::ReviewIssue {
            criterion_index: Some(criterion_index),
            criterion: criteria[criterion_index as usize - 1].clone(),
            file,
            expected: issue.expected,
            actual: issue.actual,
            suggested_change: issue.suggested_change,
            confidence: issue.confidence,
            severity: issue.severity,
            evidence_references: references,
        });
    }

    let raw_reviews = response.criterion_reviews.unwrap_or_default();
    let mut counts = BTreeMap::<u32, usize>::new();
    for review in &raw_reviews {
        *counts.entry(review.criterion_index).or_default() += 1;
    }
    let mut by_index = raw_reviews
        .into_iter()
        .filter(|review| counts.get(&review.criterion_index) == Some(&1))
        .map(|review| (review.criterion_index, review))
        .collect::<BTreeMap<_, _>>();

    let mut criterion_reviews = Vec::new();
    for index in requested_indices {
        let criterion = criteria
            .get(index as usize - 1)
            .cloned()
            .unwrap_or_default();
        let normalized = by_index.remove(&index).and_then(|review| {
            let references =
                resolve_evidence_references(&review.evidence_block_ids, evidence, authorized_paths);
            let requested_reference_count = review
                .evidence_block_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len();
            if requested_reference_count > references.as_ref().map(Vec::len).unwrap_or_default() {
                warnings.push(format!("验收项 {index} 引用了不存在或未授权的证据块"));
            }
            let has_blocker = review_issues.iter().any(|issue| {
                issue.criterion_index == Some(index)
                    && issue.severity == Some(project::ReviewIssueSeverity::Blocking)
            });
            let structurally_valid = review.confidence >= 0.7
                && references.is_some()
                && (review.conclusion != project::CriterionReviewConclusion::Unsatisfied
                    || has_blocker);
            structurally_valid.then_some(project::CriterionReviewResult {
                criterion_index: index,
                criterion: criterion.clone(),
                conclusion: review.conclusion,
                confidence: review.confidence,
                evidence_references: references.unwrap_or_default(),
            })
        });
        if counts.get(&index).copied().unwrap_or_default() > 1 {
            warnings.push(format!("验收项 {index} 返回重复结论，已按证据不足处理"));
        }
        criterion_reviews.push(normalized.unwrap_or(project::CriterionReviewResult {
            criterion_index: index,
            criterion,
            conclusion: project::CriterionReviewConclusion::EvidenceInsufficient,
            confidence: 0.0,
            evidence_references: vec![],
        }));
    }
    for index in by_index.keys() {
        warnings.push(format!("模型返回了未请求或越界的验收项 {index}，已忽略"));
    }

    let has_blocker = review_issues
        .iter()
        .any(|issue| issue.severity == Some(project::ReviewIssueSeverity::Blocking));
    let review_passed = if criteria.is_empty() {
        model_passed && !has_blocker
    } else {
        criterion_reviews
            .iter()
            .all(|review| review.conclusion == project::CriterionReviewConclusion::Satisfied)
            && !has_blocker
    };
    if model_passed != review_passed {
        warnings.push("模型总体结论与结构化逐项结果不一致，已采用后端重算结果".to_string());
    }

    project::TestResult {
        passed: review_passed,
        issues: response.issues,
        suggestion: response.suggestion,
        review_issues,
        criterion_reviews,
        warnings,
        review_passed,
        verification_stage: project::VerificationStage::Completed,
        review_status: project::ReviewStatus::Completed,
        ..Default::default()
    }
}

/// 测试
#[tauri::command]
pub(crate) async fn check_subtask(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
) -> Result<project::TestResult, String> {
    check_subtask_with_context(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        None,
        None,
        None,
        None,
    )
    .await
}

pub(crate) async fn check_subtask_with_context(
    project_path: &str,
    subtask_goal: &str,
    _subtask_id: &str,
    _milestone_id: &str,
    _mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
) -> Result<project::TestResult, String> {
    check_subtask_with_context_and_model(
        project_path,
        subtask_goal,
        _subtask_id,
        _milestone_id,
        _mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        evidence_request,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn check_subtask_with_context_and_model(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
    model_context: Option<crate::cost_ledger::ModelCallContext>,
) -> Result<project::TestResult, String> {
    check_subtask_with_context_inner(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        evidence_request,
        None,
        None,
        model_context,
    )
    .await
}

pub(crate) async fn check_subtask_with_context_and_progress(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
    progress: VerificationProgressReporter,
) -> Result<project::TestResult, String> {
    check_subtask_with_context_and_progress_and_model(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        evidence_request,
        progress,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn check_subtask_with_context_and_progress_and_model(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
    progress: VerificationProgressReporter,
    model_context: Option<crate::cost_ledger::ModelCallContext>,
) -> Result<project::TestResult, String> {
    check_subtask_with_context_inner(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        evidence_request,
        None,
        Some(progress),
        model_context,
    )
    .await
}

/// 只重新请求 AI 审查，沿用此前自动化测试事实，不再次执行测试命令。
pub(crate) async fn retry_subtask_review_with_context(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    previous_test: &project::TestResult,
    progress: VerificationProgressReporter,
) -> Result<project::TestResult, String> {
    retry_subtask_review_with_context_and_model(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        None,
        previous_test,
        progress,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn retry_subtask_review_with_context_and_model(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
    previous_test: &project::TestResult,
    progress: VerificationProgressReporter,
    model_context: Option<crate::cost_ledger::ModelCallContext>,
) -> Result<project::TestResult, String> {
    check_subtask_with_context_inner(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        evidence_request,
        Some(previous_test),
        Some(progress),
        model_context,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn review_subtask_with_context_and_model(
    project_path: &str,
    subtask_goal: &str,
    subtask_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
    previous_test: &project::TestResult,
    model_context: Option<crate::cost_ledger::ModelCallContext>,
) -> Result<project::TestResult, String> {
    check_subtask_with_context_inner(
        project_path,
        subtask_goal,
        subtask_id,
        milestone_id,
        mid_stage_id,
        acceptance_criteria,
        authorized_paths,
        execution_prompt,
        evidence_request,
        Some(previous_test),
        None,
        model_context,
    )
    .await
}

async fn check_subtask_with_context_inner(
    project_path: &str,
    subtask_goal: &str,
    _subtask_id: &str,
    _milestone_id: &str,
    _mid_stage_id: &str,
    acceptance_criteria: Option<Vec<String>>,
    authorized_paths: Option<Vec<String>>,
    execution_prompt: Option<String>,
    evidence_request: Option<ReviewEvidenceRequest>,
    previous_test: Option<&project::TestResult>,
    progress: Option<VerificationProgressReporter>,
    model_context: Option<crate::cost_ledger::ModelCallContext>,
) -> Result<project::TestResult, String> {
    if let (Some(criteria), Some(paths)) = (acceptance_criteria.as_ref(), authorized_paths.as_ref())
    {
        if let Some(local) =
            crate::validator_registry::try_validate_locally(project_path, criteria, paths)
        {
            let passed = local.review_issues.is_empty()
                && local.criterion_reviews.iter().all(|review| {
                    review.conclusion == project::CriterionReviewConclusion::Satisfied
                });
            report_verification_progress(progress.as_ref(), project::VerificationStage::Completed);
            return Ok(project::TestResult {
                passed,
                issues: local
                    .review_issues
                    .iter()
                    .map(|issue| format!("{}：{}", issue.criterion, issue.actual))
                    .collect(),
                suggestion: if passed {
                    "本地确定性验证已覆盖全部验收项".to_string()
                } else {
                    "只修复本地证据明确未满足的验收项".to_string()
                },
                review_issues: local.review_issues,
                criterion_reviews: local.criterion_reviews,
                automated_test_status: project::AutomatedTestStatus::NotConfigured,
                review_passed: passed,
                verification_kind: project::VerificationKind::DeterministicLocal,
                review_evidence_status: project::ReviewEvidenceStatus::Complete,
                review_evidence_summary: format!(
                    "本地确定性验证，无模型调用：{}",
                    local
                        .validator_runs
                        .iter()
                        .map(|run| format!(
                            "{}@{} {}",
                            run.validator, run.version, run.evidence_fingerprint
                        ))
                        .collect::<Vec<_>>()
                        .join("；")
                ),
                verification_stage: project::VerificationStage::Completed,
                review_status: project::ReviewStatus::Completed,
                ..Default::default()
            });
        }
    }
    let preparing_stage = if evidence_request
        .as_ref()
        .is_some_and(|request| request.strategy != project::ReviewEvidenceStrategy::Standard)
    {
        project::VerificationStage::TargetedEvidence
    } else {
        project::VerificationStage::PreparingEvidence
    };
    report_verification_progress(progress.as_ref(), preparing_stage);
    // 1.尝试 git diff --name-only 获取改动文件
    let changed_files = git_changed_files(project_path);

    // 2.如果 git diff 没能拿到文件列表，降级：扫描项目目录中的源文件
    let files = if let Some(paths) = authorized_paths.as_ref().filter(|paths| !paths.is_empty()) {
        authorized_review_files(changed_files, paths)
    } else if changed_files.is_empty() {
        walkdir::WalkDir::new(&project_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let path = e.path().strip_prefix(&project_path).ok()?;
                let ext = path.extension()?.to_str()?;
                // 只收集常见源代码文件
                match ext {
                    "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp"
                    | "h" | "hpp" | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "vue"
                    | "svelte" | "html" | "css" | "scss" | "json" | "yaml" | "yml" | "toml"
                    | "md" | "txt" => Some(path.to_string_lossy().to_string()),
                    _ => None,
                }
            })
            .collect::<Vec<String>>()
    } else {
        changed_files
    };

    // 3.以 Git diff 为主证据，并为长文件提供显式标记的头尾上下文。
    let evidence_request = evidence_request.unwrap_or_default();
    let review_evidence = build_review_evidence_with_request(
        project_path,
        &files,
        acceptance_criteria.as_deref().unwrap_or_default(),
        &evidence_request,
    );
    // ===== 真测试：检测项目类型，执行对应的测试命令 =====
    let test_evidence = if let Some(previous) = previous_test {
        report_verification_progress(progress.as_ref(), project::VerificationStage::ReviewRetry);
        crate::automated_validation::AutomatedTestEvidence::from_previous(previous)
    } else {
        report_verification_progress(
            progress.as_ref(),
            project::VerificationStage::AutomatedTests,
        );
        crate::automated_validation::run_project_tests(project_path)
    };
    // Mock 版本 -> 3.4.1c改动
    // 构建测试工程师prompt 的 user_message
    // eprintln!("[check_subtask] 测试结果注入完成, test_output 长度: {}",
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 构造子任务目标描述（注入给测试工程师 AI）
    let goal_section = if subtask_goal.is_empty() {
        "## 子任务目标\n（未提供子任务目标描述，请仅根据代码变更做通用质量检查）\n\n".to_string()
    } else {
        let truncated: String = subtask_goal.chars().take(2000).collect();
        let suffix = if subtask_goal.chars().count() > 2000 {
            "…（已截断）"
        } else {
            ""
        };
        format!(
            "## 子任务目标\n{}\n{}\n请根据以上目标，检查下列代码变更是否完整、正确地实现了该目标。\n\n",
            truncated, suffix
        )
    };
    let acceptance_section = acceptance_criteria
        .as_ref()
        .filter(|items| !items.is_empty())
        .map(|items| {
            let indices = criterion_indices(items, &evidence_request);
            let rendered = indices
                .iter()
                .filter_map(|index| {
                    items
                        .get(*index as usize - 1)
                        .map(|criterion| format!("{index}. {criterion}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
            let instruction =
                if evidence_request.strategy == project::ReviewEvidenceStrategy::Standard {
                    "必须逐项覆盖以下全部验收标准"
                } else {
                    "这是定向补证，只返回以下请求编号的逐项结论"
                };
            format!("## 验收标准\n{instruction}\n{rendered}\n\n")
        })
        .unwrap_or_default();
    let execution_section = execution_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
        .map(|prompt| {
            let prompt: String = prompt.chars().take(2_000).collect();
            format!("## 执行提示\n{prompt}\n\n")
        })
        .unwrap_or_default();
    let authorized_section = authorized_paths
        .as_ref()
        .filter(|paths| !paths.is_empty())
        .map(|paths| format!("## 授权文件范围\n- {}\n\n", paths.join("\n- ")))
        .unwrap_or_default();
    let review_header = format!(
        "{}{}{}{}## 审查证据状态\n{}\n证据中出现省略标记时，不得据此断言省略区域中的函数、标签或实现不存在。\n\n",
        goal_section,
        acceptance_section,
        execution_section,
        authorized_section,
        review_evidence.summary,
    );
    let user_message = if let Some(ref test_result) = test_evidence.rendered {
        format!(
            "{}请检查以下代码改动。\n\n## 自动化测试结果\n项目自动化测试已执行，结果如下：\n\n{}\n\n---\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            review_header,
            test_result,
            files.len(),
            files.join("\n"),
            review_evidence.rendered
        )
    } else {
        format!(
            "{}请检查以下代码改动：\n\n## 改动文件列表（共 {} 个文件）\n{}\n\n## 改动文件内容\n{}",
            review_header,
            files.len(),
            files.join("\n"),
            review_evidence.rendered
        )
    };
    //     test_output.as_ref().map(|s| s.len()).unwrap_or(0));
    // 调用 AI（强制 JSON 模式）
    let mut diagnosis_warnings: Vec<String> = Vec::new();
    report_verification_progress(
        progress.as_ref(),
        project::VerificationStage::RequestingReview,
    );
    let mut review_context = model_context.unwrap_or_default();
    review_context.task_id = if review_context.task_id.is_empty() {
        _subtask_id.to_string()
    } else {
        review_context.task_id
    };
    review_context.milestone_id = if review_context.milestone_id.is_empty() {
        _milestone_id.to_string()
    } else {
        review_context.milestone_id
    };
    review_context.stage_id = if review_context.stage_id.is_empty() {
        _mid_stage_id.to_string()
    } else {
        review_context.stage_id
    };
    review_context.purpose = Some(
        if evidence_request.strategy != project::ReviewEvidenceStrategy::Standard {
            crate::cost_ledger::ModelCallPurpose::EvidenceSupplement
        } else {
            crate::cost_ledger::ModelCallPurpose::Review
        },
    );
    let review_reply = crate::api::call_deepseek_api_json_typed_with_context(
        crate::prompts::TEST_PROMPT,
        &user_message,
        review_context.clone(),
    )
    .await;
    let mut review_call_id = None;
    let mut test_result: project::TestResult = match review_reply {
        Ok(response) => {
            review_call_id = Some(response.metadata.call_id.clone());
            report_verification_progress(
                progress.as_ref(),
                project::VerificationStage::ParsingReview,
            );
            match crate::review_protocol::parse_review_response_with_repair_and_progress_with_context(
                &response.content,
                progress.as_deref(),
                review_context.clone(),
            )
            .await
            {
                Ok(normalized) => {
                    let mut response = normalized.response;
                    if normalized.normalized_field_count > 0 {
                        response.warnings.push(format!(
                            "审查协议已确定性归一化 {} 个字段",
                            normalized.normalized_field_count
                        ));
                    }
                    response.warnings.extend(diagnosis_warnings);
                    let mut result = normalize_model_review(
                        response,
                        acceptance_criteria.as_deref().unwrap_or_default(),
                        authorized_paths.as_deref().unwrap_or_default(),
                        &evidence_request,
                        &review_evidence,
                    );
                    result.review_protocol_attempts =
                        u32::from(normalized.normalized_field_count > 0)
                            + u32::from(normalized.protocol_repair_attempted);
                    result
                }
                Err(e) => {
                    eprintln!(
                        "[check_subtask] 审查协议解析失败：{}，使用结构化失败结果",
                        e
                    );
                    diagnosis_warnings.push(format!("审查协议解析失败：{}", e));
                    let mut response = ModelReviewResponse::default();
                    response.issues.push("AI 审查结果协议异常".to_string());
                    response.suggestion = "重新请求代码审查".to_string();
                    response.warnings = diagnosis_warnings;
                    let mut result = normalize_model_review(
                        response,
                        acceptance_criteria.as_deref().unwrap_or_default(),
                        authorized_paths.as_deref().unwrap_or_default(),
                        &evidence_request,
                        &review_evidence,
                    );
                    result.verification_stage = if e.protocol_repair_attempted {
                        project::VerificationStage::ProtocolRepair
                    } else {
                        match &e.kind {
                            project::ReviewFailureKind::InvalidJson
                            | project::ReviewFailureKind::EmptyResponse => {
                                project::VerificationStage::ParsingReview
                            }
                            _ => project::VerificationStage::DeterministicNormalization,
                        }
                    };
                    result.review_diagnostic_summary = e.to_string();
                    result.review_status = project::ReviewStatus::Failed;
                    result.review_protocol_attempts = u32::from(e.protocol_repair_attempted);
                    result.review_failure_kind = Some(e.kind);
                    result
                }
            }
        }
        Err(error) => {
            eprintln!("[check_subtask] AI 审查请求失败：{}", error);
            project::TestResult {
                passed: false,
                review_passed: false,
                review_status: project::ReviewStatus::Failed,
                review_failure_kind: Some(error.review_failure_kind()),
                review_diagnostic_summary: error.diagnostic_summary().to_string(),
                verification_stage: project::VerificationStage::RequestingReview,
                warnings: vec!["AI 审查请求失败".to_string()],
                ..Default::default()
            }
        }
    };
    test_result.test_command = test_evidence.command;
    test_result.test_exit_code = test_evidence.exit_code;
    test_result.test_output_summary = test_evidence.output_summary;
    test_result.automated_test_status = test_evidence.status.clone();
    test_result.verification_kind = match test_evidence.status {
        project::AutomatedTestStatus::Passed | project::AutomatedTestStatus::Failed => {
            project::VerificationKind::AutomatedTestAndReview
        }
        project::AutomatedTestStatus::NotConfigured | project::AutomatedTestStatus::Unknown => {
            project::VerificationKind::CodeReviewOnly
        }
        project::AutomatedTestStatus::Unavailable => project::VerificationKind::Legacy,
    };
    test_result.review_evidence_status = review_evidence.status;
    test_result.review_evidence_summary = review_evidence.summary;

    match test_evidence.status {
        project::AutomatedTestStatus::Failed => {
            test_result.passed = false;
            if !test_result
                .issues
                .iter()
                .any(|issue| issue.contains("自动化测试失败"))
            {
                test_result.issues.push("自动化测试失败".to_string());
            }
        }
        project::AutomatedTestStatus::Unavailable => {
            test_result.passed = false;
            if !test_result
                .issues
                .iter()
                .any(|issue| issue.contains("测试环境不可用"))
            {
                test_result.issues.push("测试环境不可用".to_string());
            }
        }
        _ => {}
    }

    report_verification_progress(progress.as_ref(), test_result.verification_stage.clone());

    if let Some(call_id) = review_call_id.as_deref() {
        crate::cost_ledger::mark_call_outcome_best_effort(
            &review_context.project_name,
            call_id,
            crate::cost_ledger::ModelCallOutcome {
                produced_evidence: !test_result.criterion_reviews.is_empty(),
                produced_fact: test_result.review_status == project::ReviewStatus::Completed,
                ..Default::default()
            },
        );
    }

    Ok(test_result)
}
