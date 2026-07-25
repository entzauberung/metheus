use crate::project;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QualityGateOutcome {
    Passed,
    CodeUnsatisfied,
    EvidenceInsufficient,
    ContractConflict,
    ReviewOscillation,
    TestUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityGateEvaluation {
    pub(crate) outcome: QualityGateOutcome,
    pub(crate) message: String,
}

impl QualityGateEvaluation {
    pub(crate) fn passed(&self) -> bool {
        self.outcome == QualityGateOutcome::Passed
    }

    pub(crate) fn recovery_error_kind(
        &self,
        test: Option<&project::TestResult>,
    ) -> project::RecoveryErrorKind {
        match self.outcome {
            QualityGateOutcome::Passed => project::RecoveryErrorKind::HumanRequired,
            QualityGateOutcome::CodeUnsatisfied
                if test.is_some_and(|test| {
                    test.automated_test_status == project::AutomatedTestStatus::Failed
                }) =>
            {
                project::RecoveryErrorKind::TestFailure
            }
            QualityGateOutcome::CodeUnsatisfied => project::RecoveryErrorKind::ReviewFailure,
            QualityGateOutcome::EvidenceInsufficient => {
                project::RecoveryErrorKind::EvidenceInsufficient
            }
            QualityGateOutcome::ContractConflict => {
                project::RecoveryErrorKind::ContractContradiction
            }
            QualityGateOutcome::ReviewOscillation => {
                project::RecoveryErrorKind::ValidationOscillation
            }
            QualityGateOutcome::TestUnavailable => project::RecoveryErrorKind::TestUnavailable,
        }
    }
}

fn evaluation(outcome: QualityGateOutcome, message: impl Into<String>) -> QualityGateEvaluation {
    QualityGateEvaluation {
        outcome,
        message: message.into(),
    }
}

fn review_service_unavailable(test: &project::TestResult) -> bool {
    test.review_evidence_status == project::ReviewEvidenceStatus::Unavailable
        || test
            .warnings
            .iter()
            .any(|warning| warning.contains("AI API") || warning.contains("解析失败"))
}

pub(crate) fn evaluate(
    test: Option<&project::TestResult>,
    ledger: &[project::AcceptanceLedgerItem],
    criterion_count: usize,
    review_oscillation: bool,
) -> QualityGateEvaluation {
    let Some(test) = test else {
        return evaluation(
            QualityGateOutcome::TestUnavailable,
            "缺少测试与代码审查结果",
        );
    };

    if test.automated_test_status == project::AutomatedTestStatus::Failed {
        return evaluation(
            QualityGateOutcome::CodeUnsatisfied,
            "自动化测试明确失败，质量门禁阻断",
        );
    }
    if test.automated_test_status == project::AutomatedTestStatus::Unavailable
        || review_service_unavailable(test)
    {
        return evaluation(
            QualityGateOutcome::TestUnavailable,
            "测试或代码审查服务不可用",
        );
    }
    if review_oscillation {
        return evaluation(
            QualityGateOutcome::ReviewOscillation,
            "同一验收项在连续审查中反复改变结论",
        );
    }
    if ledger
        .iter()
        .any(|item| item.status == project::AcceptanceStatus::Contradictory)
    {
        return evaluation(
            QualityGateOutcome::ContractConflict,
            "逐项结论与有效阻断证据互相冲突",
        );
    }
    if criterion_count != ledger.len()
        || ledger
            .iter()
            .any(|item| item.status == project::AcceptanceStatus::Unknown)
    {
        return evaluation(
            QualityGateOutcome::EvidenceInsufficient,
            "验收账本不完整或存在未证明项",
        );
    }
    if ledger
        .iter()
        .any(|item| item.status == project::AcceptanceStatus::Unsatisfied)
    {
        return evaluation(
            QualityGateOutcome::CodeUnsatisfied,
            "存在带有效阻断证据的未满足验收项",
        );
    }
    if criterion_count > 0 {
        return evaluation(QualityGateOutcome::Passed, "逐验收项质量门禁通过");
    }

    let has_blocking_review_issue = test.review_issues.iter().any(|issue| {
        issue.severity == Some(project::ReviewIssueSeverity::Blocking)
            && !issue.evidence_references.is_empty()
    });
    if has_blocking_review_issue {
        return evaluation(
            QualityGateOutcome::CodeUnsatisfied,
            "代码审查发现带有效证据的阻断问题",
        );
    }
    if test.review_passed
        || (test.verification_kind == project::VerificationKind::Legacy && test.passed)
    {
        return evaluation(QualityGateOutcome::Passed, "代码审查质量门禁通过");
    }

    evaluation(
        QualityGateOutcome::EvidenceInsufficient,
        "旧结果无法形成可靠的逐项质量结论",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(status: project::AcceptanceStatus) -> Vec<project::AcceptanceLedgerItem> {
        vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            status,
            ..Default::default()
        }]
    }

    fn reviewed_test() -> project::TestResult {
        project::TestResult {
            review_passed: true,
            automated_test_status: project::AutomatedTestStatus::NotConfigured,
            verification_kind: project::VerificationKind::CodeReviewOnly,
            review_evidence_status: project::ReviewEvidenceStatus::Partial,
            ..Default::default()
        }
    }

    #[test]
    fn partial_global_evidence_with_satisfied_ledger_passes() {
        let test = reviewed_test();
        let result = evaluate(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
        );
        assert_eq!(result.outcome, QualityGateOutcome::Passed);
    }

    #[test]
    fn warnings_and_suggestions_do_not_block() {
        let mut test = reviewed_test();
        test.warnings.push("ordinary warning".to_string());
        test.review_issues.push(project::ReviewIssue {
            severity: Some(project::ReviewIssueSeverity::Suggestion),
            ..Default::default()
        });
        assert!(evaluate(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
        )
        .passed());
    }

    #[test]
    fn explicit_automated_test_failure_always_blocks() {
        let mut test = reviewed_test();
        test.automated_test_status = project::AutomatedTestStatus::Failed;
        let result = evaluate(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
        );
        assert_eq!(result.outcome, QualityGateOutcome::CodeUnsatisfied);
        assert_eq!(
            result.recovery_error_kind(Some(&test)),
            project::RecoveryErrorKind::TestFailure
        );
    }

    #[test]
    fn unknown_contradictory_and_oscillating_are_distinct() {
        let test = reviewed_test();
        assert_eq!(
            evaluate(
                Some(&test),
                &ledger(project::AcceptanceStatus::Unknown),
                1,
                false,
            )
            .outcome,
            QualityGateOutcome::EvidenceInsufficient
        );
        assert_eq!(
            evaluate(
                Some(&test),
                &ledger(project::AcceptanceStatus::Contradictory),
                1,
                false,
            )
            .outcome,
            QualityGateOutcome::ContractConflict
        );
        assert_eq!(
            evaluate(
                Some(&test),
                &ledger(project::AcceptanceStatus::Satisfied),
                1,
                true,
            )
            .outcome,
            QualityGateOutcome::ReviewOscillation
        );
    }
}
