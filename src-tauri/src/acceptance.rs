use crate::project;
use std::collections::BTreeMap;

pub(crate) fn build_ledger(
    criteria: &[String],
    result: &project::TestResult,
    authorized_paths: &[String],
) -> Vec<project::AcceptanceLedgerItem> {
    let now = chrono::Utc::now().to_rfc3339();
    let review_by_index = result
        .review_issues
        .iter()
        .filter(|issue| {
            issue.confidence >= 0.7
                && !issue.expected.trim().is_empty()
                && !issue.actual.trim().is_empty()
                && !issue.suggested_change.trim().is_empty()
                && authorized_paths.iter().any(|path| path == &issue.file)
        })
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
            {
                (
                    project::AcceptanceStatus::Satisfied,
                    result.review_evidence_summary.clone(),
                    1.0,
                )
            } else {
                // Partial/unavailable evidence never proves absence.
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
            .all(|item| item.status == project::AcceptanceStatus::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
