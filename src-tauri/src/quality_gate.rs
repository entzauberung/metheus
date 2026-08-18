use crate::project;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QualityGateOutcome {
    Passed,
    CodeUnsatisfied,
    EvidenceInsufficient,
    ContractConflict,
    ReviewOscillation,
    AutomatedTestUnavailable,
    ReviewTransientFailure,
    ReviewProtocolFailure,
    ReviewServiceBlocked,
    /// 旧结果缺少结构化状态时的兼容值。
    TestUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityGateEvaluation {
    pub(crate) outcome: QualityGateOutcome,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletionDecision {
    /// Execution, quality and ledger facts are ready for the confirmation transaction.
    AwaitingConfirmation,
    /// The confirmation transaction has reached the required stage.
    Completed,
    /// Completion is forbidden until the reported reason is resolved.
    Blocked(String),
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
            QualityGateOutcome::AutomatedTestUnavailable => {
                project::RecoveryErrorKind::AutomatedTestUnavailable
            }
            QualityGateOutcome::ReviewTransientFailure => {
                project::RecoveryErrorKind::ReviewTransientFailure
            }
            QualityGateOutcome::ReviewProtocolFailure => {
                project::RecoveryErrorKind::ReviewProtocolFailure
            }
            QualityGateOutcome::ReviewServiceBlocked => {
                project::RecoveryErrorKind::ReviewServiceBlocked
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

pub(crate) fn evaluate(
    test: Option<&project::TestResult>,
    ledger: &[project::AcceptanceLedgerItem],
    criterion_count: usize,
    review_oscillation: bool,
) -> QualityGateEvaluation {
    evaluate_with_deferred(test, ledger, criterion_count, review_oscillation, false)
}

pub(crate) fn evaluate_with_deferred(
    test: Option<&project::TestResult>,
    ledger: &[project::AcceptanceLedgerItem],
    criterion_count: usize,
    review_oscillation: bool,
    allow_matching_deferred_human_review: bool,
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
    if test.automated_test_status == project::AutomatedTestStatus::Unavailable {
        return evaluation(
            QualityGateOutcome::AutomatedTestUnavailable,
            "自动化测试环境不可用",
        );
    }
    if test.review_status == project::ReviewStatus::Failed {
        return match test.review_failure_kind.as_ref() {
            Some(
                project::ReviewFailureKind::Network
                | project::ReviewFailureKind::Timeout
                | project::ReviewFailureKind::RateLimited
                | project::ReviewFailureKind::ServiceUnavailable,
            ) => evaluation(
                QualityGateOutcome::ReviewTransientFailure,
                "AI 审查服务暂时不可用",
            ),
            Some(
                project::ReviewFailureKind::EmptyResponse
                | project::ReviewFailureKind::InvalidJson
                | project::ReviewFailureKind::FieldTypeMismatch,
            ) => evaluation(
                QualityGateOutcome::ReviewProtocolFailure,
                "AI 审查结果未通过协议校验",
            ),
            Some(
                project::ReviewFailureKind::Authentication
                | project::ReviewFailureKind::QuotaExceeded,
            ) => evaluation(
                QualityGateOutcome::ReviewServiceBlocked,
                "AI 审查服务因认证或额度问题阻断",
            ),
            None => evaluation(
                QualityGateOutcome::TestUnavailable,
                "旧审查结果缺少结构化失败分类",
            ),
        };
    }
    if test.review_evidence_status == project::ReviewEvidenceStatus::Unavailable {
        return evaluation(
            QualityGateOutcome::EvidenceInsufficient,
            "代码审查证据不可用",
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
    let has_deferred = ledger.iter().any(|item| {
        matches!(
            item.status,
            project::AcceptanceStatus::AiProvisionallySatisfied
                | project::AcceptanceStatus::DeferredHumanReview
        )
    });
    if has_deferred && !allow_matching_deferred_human_review {
        return evaluation(
            QualityGateOutcome::EvidenceInsufficient,
            "AI 临时结论缺少匹配的大阶段人工确认清单",
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

fn ledger_blocker(subtask: &project::Subtask) -> Option<&'static str> {
    let criterion_count = subtask.acceptance_criteria.len();
    if criterion_count != subtask.acceptance_ledger.len() {
        return Some("验收账本未初始化或不完整");
    }
    if subtask
        .acceptance_ledger
        .iter()
        .any(|item| item.status == project::AcceptanceStatus::Contradictory)
    {
        return Some("逐项结论与阻断证据互相冲突（Contradictory）");
    }
    if subtask
        .acceptance_ledger
        .iter()
        .any(|item| item.status == project::AcceptanceStatus::Unknown)
    {
        return Some("验收账本不完整或存在未证明项（Unknown）");
    }
    if subtask
        .acceptance_ledger
        .iter()
        .any(|item| item.status == project::AcceptanceStatus::Unsatisfied)
    {
        return Some("存在带有效阻断证据的未满足验收项（Unsatisfied）");
    }
    None
}

/// Single backend completion ruling shared by confirmation and scheduling paths.
/// `confirmation_reached` is supplied only by the durable Git/confirmation transaction.
pub(crate) fn decide_completion(
    subtask: &project::Subtask,
    quality: Option<&QualityGateEvaluation>,
    confirmation_reached: bool,
) -> CompletionDecision {
    let Some(execution) = subtask.execution_result.as_ref() else {
        return CompletionDecision::Blocked("缺少执行结果，禁止完成".to_string());
    };
    if !execution.success {
        return CompletionDecision::Blocked("执行结果未成功，禁止完成".to_string());
    }
    if let Some(reason) = ledger_blocker(subtask) {
        return CompletionDecision::Blocked(reason.to_string());
    }
    let Some(quality) = quality else {
        return CompletionDecision::Blocked("缺少质量门结果，禁止完成".to_string());
    };
    if !quality.passed() {
        return CompletionDecision::Blocked(quality.message.clone());
    }
    if !confirmation_reached {
        return CompletionDecision::AwaitingConfirmation;
    }
    CompletionDecision::Completed
}

/// Cheap guard for dispatchers that may only schedule the confirmation action.
/// It deliberately does not replace the full quality evaluation.
pub(crate) fn confirmation_prerequisites(subtask: &project::Subtask) -> Result<(), String> {
    let Some(execution) = subtask.execution_result.as_ref() else {
        return Err("缺少执行结果，禁止进入确认事务".to_string());
    };
    if !execution.success {
        return Err("执行结果未成功，禁止进入确认事务".to_string());
    }
    if let Some(reason) = ledger_blocker(subtask) {
        return Err(reason.to_string());
    }
    if subtask.test_result.is_none()
        && !subtask
            .human_verification
            .as_ref()
            .is_some_and(|verification| {
                matches!(
                    verification.resolution,
                    project::HumanResolution::ConfirmActualPass
                        | project::HumanResolution::AcceptDeviation
                )
            })
    {
        return Err("缺少质量门结果，禁止进入确认事务".to_string());
    }
    Ok(())
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
            review_status: project::ReviewStatus::Completed,
            ..Default::default()
        }
    }

    #[test]
    fn structured_review_failures_do_not_parse_warning_text() {
        let mut test = reviewed_test();
        test.warnings
            .push("AI API 调用失败：仅为旧诊断文本".to_string());
        assert!(evaluate(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
        )
        .passed());

        test.review_status = project::ReviewStatus::Failed;
        for kind in [
            project::ReviewFailureKind::Network,
            project::ReviewFailureKind::Timeout,
            project::ReviewFailureKind::RateLimited,
            project::ReviewFailureKind::ServiceUnavailable,
        ] {
            test.review_failure_kind = Some(kind);
            assert_eq!(
                evaluate(Some(&test), &[], 0, false).outcome,
                QualityGateOutcome::ReviewTransientFailure
            );
        }
        for kind in [
            project::ReviewFailureKind::EmptyResponse,
            project::ReviewFailureKind::InvalidJson,
            project::ReviewFailureKind::FieldTypeMismatch,
        ] {
            test.review_failure_kind = Some(kind);
            assert_eq!(
                evaluate(Some(&test), &[], 0, false).outcome,
                QualityGateOutcome::ReviewProtocolFailure
            );
        }
        for kind in [
            project::ReviewFailureKind::Authentication,
            project::ReviewFailureKind::QuotaExceeded,
        ] {
            test.review_failure_kind = Some(kind);
            assert_eq!(
                evaluate(Some(&test), &[], 0, false).outcome,
                QualityGateOutcome::ReviewServiceBlocked
            );
        }
    }

    #[test]
    fn automated_test_unavailable_is_distinct_from_review_evidence() {
        let mut test = reviewed_test();
        test.automated_test_status = project::AutomatedTestStatus::Unavailable;
        assert_eq!(
            evaluate(Some(&test), &[], 0, false).outcome,
            QualityGateOutcome::AutomatedTestUnavailable
        );

        test.automated_test_status = project::AutomatedTestStatus::NotConfigured;
        test.review_evidence_status = project::ReviewEvidenceStatus::Unavailable;
        assert_eq!(
            evaluate(Some(&test), &[], 0, false).outcome,
            QualityGateOutcome::EvidenceInsufficient
        );
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
        let result = evaluate_with_deferred(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
            true,
        );
        assert_eq!(result.outcome, QualityGateOutcome::CodeUnsatisfied);
        assert_eq!(
            result.recovery_error_kind(Some(&test)),
            project::RecoveryErrorKind::TestFailure
        );
    }

    #[test]
    fn deferred_human_status_only_passes_the_explicit_matching_batch_path() {
        let test = reviewed_test();
        let deferred = ledger(project::AcceptanceStatus::DeferredHumanReview);

        assert_eq!(
            evaluate(Some(&test), &deferred, 1, false).outcome,
            QualityGateOutcome::EvidenceInsufficient
        );
        assert_eq!(
            evaluate_with_deferred(Some(&test), &deferred, 1, false, true).outcome,
            QualityGateOutcome::Passed
        );
        assert_eq!(
            evaluate_with_deferred(
                Some(&test),
                &ledger(project::AcceptanceStatus::Unsatisfied),
                1,
                false,
                true,
            )
            .outcome,
            QualityGateOutcome::CodeUnsatisfied
        );
        assert_eq!(
            evaluate_with_deferred(
                Some(&test),
                &ledger(project::AcceptanceStatus::Unknown),
                1,
                false,
                true,
            )
            .outcome,
            QualityGateOutcome::EvidenceInsufficient
        );
        assert_eq!(
            evaluate_with_deferred(
                Some(&test),
                &ledger(project::AcceptanceStatus::Contradictory),
                1,
                false,
                true,
            )
            .outcome,
            QualityGateOutcome::ContractConflict
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

    #[test]
    fn completion_requires_execution_quality_ledger_and_confirmation() {
        let test = reviewed_test();
        let quality = evaluate(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
        );
        let mut subtask = project::Subtask::default();
        subtask.acceptance_criteria = vec!["criterion".to_string()];
        subtask.acceptance_ledger = ledger(project::AcceptanceStatus::Satisfied);
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });

        assert_eq!(
            decide_completion(&subtask, Some(&quality), false),
            CompletionDecision::AwaitingConfirmation
        );
        assert_eq!(
            decide_completion(&subtask, Some(&quality), true),
            CompletionDecision::Completed
        );

        subtask.execution_result.as_mut().unwrap().success = false;
        assert!(matches!(
            decide_completion(&subtask, Some(&quality), true),
            CompletionDecision::Blocked(reason) if reason.contains("执行结果未成功")
        ));
    }

    #[test]
    fn completion_rejects_unknown_unsatisfied_contradictory_and_missing_ledger() {
        let test = reviewed_test();
        let quality = evaluate(
            Some(&test),
            &ledger(project::AcceptanceStatus::Satisfied),
            1,
            false,
        );
        for status in [
            project::AcceptanceStatus::Unknown,
            project::AcceptanceStatus::Unsatisfied,
            project::AcceptanceStatus::Contradictory,
        ] {
            let mut subtask = project::Subtask::default();
            subtask.acceptance_criteria = vec!["criterion".to_string()];
            subtask.acceptance_ledger = ledger(status);
            subtask.execution_result = Some(project::ExecutionResult {
                success: true,
                ..Default::default()
            });
            assert!(matches!(
                decide_completion(&subtask, Some(&quality), true),
                CompletionDecision::Blocked(_)
            ));
        }

        let mut missing = project::Subtask::default();
        missing.acceptance_criteria = vec!["criterion".to_string()];
        missing.execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        assert!(matches!(
            decide_completion(&missing, Some(&quality), true),
            CompletionDecision::Blocked(reason) if reason.contains("账本")
        ));
    }

    #[test]
    fn confirmation_prerequisites_require_execution_ledger_and_quality_facts() {
        let mut subtask = project::Subtask::default();
        subtask.acceptance_criteria = vec!["criterion".to_string()];
        subtask.acceptance_ledger = ledger(project::AcceptanceStatus::Satisfied);
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });

        assert!(matches!(
            confirmation_prerequisites(&subtask),
            Err(reason) if reason.contains("质量门")
        ));
        subtask.test_result = Some(reviewed_test());
        assert!(confirmation_prerequisites(&subtask).is_ok());

        subtask.execution_result.as_mut().unwrap().success = false;
        assert!(matches!(
            confirmation_prerequisites(&subtask),
            Err(reason) if reason.contains("执行结果")
        ));
    }
}
