use crate::project;
use std::collections::BTreeMap;

fn valid_evidence_references(
    references: &[project::ReviewEvidenceReference],
    authorized_paths: &[String],
) -> bool {
    !references.is_empty()
        && references.iter().all(|reference| {
            !reference.block_id.trim().is_empty()
                && authorized_paths.iter().any(|path| path == &reference.file)
                && match (reference.start_line, reference.end_line) {
                    (Some(start), Some(end)) => start > 0 && end >= start,
                    (None, None) => true,
                    _ => false,
                }
        })
}

fn valid_blocking_issue(
    issue: &project::ReviewIssue,
    criterion_index: u32,
    authorized_paths: &[String],
) -> bool {
    issue.criterion_index == Some(criterion_index)
        && issue.severity == Some(project::ReviewIssueSeverity::Blocking)
        && issue.confidence >= 0.7
        && !issue.expected.trim().is_empty()
        && !issue.actual.trim().is_empty()
        && !issue.suggested_change.trim().is_empty()
        && authorized_paths.iter().any(|path| path == &issue.file)
        && valid_evidence_references(&issue.evidence_references, authorized_paths)
}

fn legacy_issue_is_actionable(issue: &project::ReviewIssue, authorized_paths: &[String]) -> bool {
    let evidence_is_valid = match issue.severity {
        Some(project::ReviewIssueSeverity::Blocking) => {
            valid_evidence_references(&issue.evidence_references, authorized_paths)
        }
        None => true,
        Some(project::ReviewIssueSeverity::Warning)
        | Some(project::ReviewIssueSeverity::Suggestion) => false,
    };
    issue.confidence >= 0.7
        && evidence_is_valid
        && !issue.expected.trim().is_empty()
        && !issue.actual.trim().is_empty()
        && !issue.suggested_change.trim().is_empty()
        && authorized_paths.iter().any(|path| path == &issue.file)
}

fn build_legacy_ledger(
    criteria: &[String],
    result: &project::TestResult,
    authorized_paths: &[String],
    now: &str,
) -> Vec<project::AcceptanceLedgerItem> {
    let review_by_index = result
        .review_issues
        .iter()
        .filter(|issue| legacy_issue_is_actionable(issue, authorized_paths))
        .filter_map(|issue| issue.criterion_index.map(|index| (index, issue)))
        .collect::<BTreeMap<_, _>>();

    criteria
        .iter()
        .enumerate()
        .map(|(index, criterion)| {
            let criterion_index = index as u32 + 1;
            let issue = review_by_index.get(&criterion_index);
            let (status, evidence, confidence) = if let Some(issue) = issue {
                (
                    project::AcceptanceStatus::Unsatisfied,
                    format!("expected={}；actual={}", issue.expected, issue.actual),
                    issue.confidence,
                )
            } else if result.passed
                && result.review_evidence_status == project::ReviewEvidenceStatus::Complete
                && result.review_issues.iter().all(|issue| {
                    issue.severity == Some(project::ReviewIssueSeverity::Warning)
                        || issue.severity == Some(project::ReviewIssueSeverity::Suggestion)
                })
            {
                (
                    project::AcceptanceStatus::Satisfied,
                    result.review_evidence_summary.clone(),
                    1.0,
                )
            } else {
                (
                    project::AcceptanceStatus::Unknown,
                    result.review_evidence_summary.clone(),
                    0.0,
                )
            };
            project::AcceptanceLedgerItem {
                criterion_index,
                criterion: criterion.clone(),
                status,
                evidence,
                evidence_references: issue
                    .map(|issue| issue.evidence_references.clone())
                    .unwrap_or_default(),
                confidence,
                updated_at: now.to_string(),
            }
        })
        .collect()
}

pub(crate) fn build_ledger(
    criteria: &[String],
    result: &project::TestResult,
    authorized_paths: &[String],
) -> Vec<project::AcceptanceLedgerItem> {
    let now = chrono::Utc::now().to_rfc3339();
    if result.criterion_reviews.is_empty() {
        return build_legacy_ledger(criteria, result, authorized_paths, &now);
    }

    let mut review_counts = BTreeMap::<u32, usize>::new();
    for review in &result.criterion_reviews {
        *review_counts.entry(review.criterion_index).or_default() += 1;
    }
    let reviews = result
        .criterion_reviews
        .iter()
        .filter(|review| review_counts.get(&review.criterion_index) == Some(&1))
        .map(|review| (review.criterion_index, review))
        .collect::<BTreeMap<_, _>>();

    criteria
        .iter()
        .enumerate()
        .map(|(index, criterion)| {
            let criterion_index = index as u32 + 1;
            let review = reviews.get(&criterion_index).copied();
            let blocking_issue = result
                .review_issues
                .iter()
                .find(|issue| valid_blocking_issue(issue, criterion_index, authorized_paths));
            let review_has_evidence = review.is_some_and(|review| {
                review.confidence >= 0.7
                    && valid_evidence_references(&review.evidence_references, authorized_paths)
            });
            let (status, evidence, evidence_references, confidence) = match (review, blocking_issue)
            {
                (Some(review), Some(issue))
                    if review_has_evidence
                        && review.conclusion == project::CriterionReviewConclusion::Satisfied =>
                {
                    (
                        project::AcceptanceStatus::Contradictory,
                        format!(
                            "逐项结论为满足，但存在有效阻断证据：expected={}；actual={}",
                            issue.expected, issue.actual
                        ),
                        issue.evidence_references.clone(),
                        review.confidence.min(issue.confidence),
                    )
                }
                (Some(review), _)
                    if review_has_evidence
                        && review.conclusion == project::CriterionReviewConclusion::Satisfied =>
                {
                    (
                        project::AcceptanceStatus::Satisfied,
                        format!("逐项审查已满足：{}", criterion),
                        review.evidence_references.clone(),
                        review.confidence,
                    )
                }
                (Some(review), Some(issue))
                    if review_has_evidence
                        && review.conclusion == project::CriterionReviewConclusion::Unsatisfied =>
                {
                    (
                        project::AcceptanceStatus::Unsatisfied,
                        format!("expected={}；actual={}", issue.expected, issue.actual),
                        issue.evidence_references.clone(),
                        review.confidence.min(issue.confidence),
                    )
                }
                _ => (
                    project::AcceptanceStatus::Unknown,
                    result.review_evidence_summary.clone(),
                    vec![],
                    0.0,
                ),
            };
            project::AcceptanceLedgerItem {
                criterion_index,
                criterion: criterion.clone(),
                status,
                evidence,
                evidence_references,
                confidence,
                updated_at: now.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn actionable_issues(
    ledger: &[project::AcceptanceLedgerItem],
) -> Vec<&project::AcceptanceLedgerItem> {
    ledger
        .iter()
        .filter(|item| item.status == project::AcceptanceStatus::Unsatisfied)
        .collect()
}

pub(crate) fn needs_evidence(ledger: &[project::AcceptanceLedgerItem]) -> bool {
    !ledger.is_empty()
        && ledger
            .iter()
            .any(|item| item.status == project::AcceptanceStatus::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence_reference() -> project::ReviewEvidenceReference {
        project::ReviewEvidenceReference {
            block_id: "E001".to_string(),
            source_kind: project::EvidenceSourceKind::CurrentFileSnippet,
            file: "index.html".to_string(),
            start_line: Some(1),
            end_line: Some(3),
        }
    }

    fn criterion_review(
        conclusion: project::CriterionReviewConclusion,
    ) -> project::CriterionReviewResult {
        project::CriterionReviewResult {
            criterion_index: 1,
            criterion: "bind dragstart".to_string(),
            conclusion,
            confidence: 0.9,
            evidence_references: vec![evidence_reference()],
        }
    }

    #[test]
    fn partial_evidence_is_unknown_not_unsatisfied() {
        let result = project::TestResult {
            passed: false,
            review_evidence_status: project::ReviewEvidenceStatus::Partial,
            review_evidence_summary: "file truncated".to_string(),
            ..Default::default()
        };
        let ledger = build_ledger(
            &["bind dragstart".to_string()],
            &result,
            &["index.html".to_string()],
        );
        assert_eq!(ledger[0].status, project::AcceptanceStatus::Unknown);
        assert!(actionable_issues(&ledger).is_empty());
        assert!(needs_evidence(&ledger));
    }

    #[test]
    fn one_unknown_criterion_requires_evidence_rebuild() {
        let result = project::TestResult {
            passed: true,
            review_evidence_status: project::ReviewEvidenceStatus::Complete,
            review_issues: vec![project::ReviewIssue {
                criterion_index: Some(1),
                file: "index.html".to_string(),
                expected: "a".to_string(),
                actual: "b".to_string(),
                suggested_change: "fix".to_string(),
                confidence: 0.9,
                ..Default::default()
            }],
            ..Default::default()
        };
        let ledger = build_ledger(
            &["mapped".to_string(), "unmapped".to_string()],
            &result,
            &["index.html".to_string()],
        );
        assert_eq!(ledger[0].status, project::AcceptanceStatus::Unsatisfied);
        assert_eq!(ledger[1].status, project::AcceptanceStatus::Unknown);
        assert!(needs_evidence(&ledger));
    }

    #[test]
    fn low_confidence_or_out_of_scope_issue_is_not_actionable() {
        let result = project::TestResult {
            review_evidence_status: project::ReviewEvidenceStatus::Complete,
            review_issues: vec![project::ReviewIssue {
                criterion_index: Some(1),
                file: "other.html".to_string(),
                expected: "a".to_string(),
                actual: "b".to_string(),
                suggested_change: "c".to_string(),
                confidence: 0.6,
                ..Default::default()
            }],
            ..Default::default()
        };
        let ledger = build_ledger(
            &["criterion".to_string()],
            &result,
            &["index.html".to_string()],
        );
        assert_eq!(ledger[0].status, project::AcceptanceStatus::Unknown);
        assert!(needs_evidence(&ledger));
    }

    #[test]
    fn structured_satisfied_criterion_overrides_global_partial_evidence() {
        let result = project::TestResult {
            passed: false,
            review_evidence_status: project::ReviewEvidenceStatus::Partial,
            review_evidence_summary: "large file partially expanded".to_string(),
            criterion_reviews: vec![criterion_review(
                project::CriterionReviewConclusion::Satisfied,
            )],
            review_issues: vec![project::ReviewIssue {
                criterion_index: Some(1),
                file: "index.html".to_string(),
                actual: "可以简化命名".to_string(),
                confidence: 0.9,
                severity: Some(project::ReviewIssueSeverity::Suggestion),
                ..Default::default()
            }],
            ..Default::default()
        };

        let ledger = build_ledger(
            &["bind dragstart".to_string()],
            &result,
            &["index.html".to_string()],
        );

        assert_eq!(ledger[0].status, project::AcceptanceStatus::Satisfied);
        assert!(!needs_evidence(&ledger));
    }

    #[test]
    fn structured_unsatisfied_requires_valid_blocking_evidence() {
        let result = project::TestResult {
            criterion_reviews: vec![criterion_review(
                project::CriterionReviewConclusion::Unsatisfied,
            )],
            review_issues: vec![project::ReviewIssue {
                criterion_index: Some(1),
                file: "index.html".to_string(),
                expected: "dragstart bound".to_string(),
                actual: "handler absent".to_string(),
                suggested_change: "bind handler".to_string(),
                confidence: 0.9,
                severity: Some(project::ReviewIssueSeverity::Blocking),
                evidence_references: vec![],
                ..Default::default()
            }],
            ..Default::default()
        };

        let ledger = build_ledger(
            &["bind dragstart".to_string()],
            &result,
            &["index.html".to_string()],
        );

        assert_eq!(ledger[0].status, project::AcceptanceStatus::Unknown);
        assert!(actionable_issues(&ledger).is_empty());
    }

    #[test]
    fn satisfied_criterion_with_blocker_is_contract_contradiction() {
        let reference = evidence_reference();
        let result = project::TestResult {
            criterion_reviews: vec![criterion_review(
                project::CriterionReviewConclusion::Satisfied,
            )],
            review_issues: vec![project::ReviewIssue {
                criterion_index: Some(1),
                file: "index.html".to_string(),
                expected: "dragstart bound".to_string(),
                actual: "handler absent".to_string(),
                suggested_change: "bind handler".to_string(),
                confidence: 0.9,
                severity: Some(project::ReviewIssueSeverity::Blocking),
                evidence_references: vec![reference],
                ..Default::default()
            }],
            ..Default::default()
        };

        let ledger = build_ledger(
            &["bind dragstart".to_string()],
            &result,
            &["index.html".to_string()],
        );

        assert_eq!(ledger[0].status, project::AcceptanceStatus::Contradictory);
        assert!(actionable_issues(&ledger).is_empty());
        assert!(!needs_evidence(&ledger));
    }

    #[test]
    fn duplicate_structured_criterion_is_unknown() {
        let review = criterion_review(project::CriterionReviewConclusion::Satisfied);
        let result = project::TestResult {
            criterion_reviews: vec![review.clone(), review],
            ..Default::default()
        };

        let ledger = build_ledger(
            &["bind dragstart".to_string()],
            &result,
            &["index.html".to_string()],
        );

        assert_eq!(ledger[0].status, project::AcceptanceStatus::Unknown);
        assert!(needs_evidence(&ledger));
    }
}
