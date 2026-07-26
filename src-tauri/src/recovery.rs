use crate::pipeline::{self, PipelineState, PipelineStatus, SubtaskStatusItem};
use crate::project;
use crate::AppState;
use std::collections::{BTreeMap, BTreeSet};

const MAX_DIAGNOSIS_CHARS: usize = 12_000;
const MAX_EVIDENCE_CHARS: usize = 6_000;
const MAX_FAILURE_HISTORY: usize = 4;
const DEFAULT_MAX_ATTEMPTS: u32 = 2;
const MAX_EVIDENCE_REBUILD_ATTEMPTS: u32 = 2;
const MAX_TRANSIENT_REVIEW_RETRIES: u32 = 3;
const MAX_PROTOCOL_REVIEW_RETRIES: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryRetestKind {
    Full,
    ReviewOnly,
}

#[allow(clippy::too_many_arguments)]
fn write_recovery_history(
    proj: &mut project::Project,
    level: &str,
    event_type: project::ExecutionEventType,
    text: String,
    milestone_id: Option<&str>,
    mid_stage_id: Option<&str>,
    subtask_id: Option<&str>,
) {
    pipeline::write_execution_history_with_source(
        proj,
        level,
        event_type,
        project::OperationSource::Recovery,
        text,
        milestone_id,
        mid_stage_id,
        subtask_id,
    );
}

fn validation_retry_limit(kind: &project::RecoveryErrorKind) -> Option<u32> {
    match kind {
        project::RecoveryErrorKind::ReviewTransientFailure => Some(MAX_TRANSIENT_REVIEW_RETRIES),
        project::RecoveryErrorKind::ReviewProtocolFailure => Some(MAX_PROTOCOL_REVIEW_RETRIES),
        _ => None,
    }
}

fn validation_retry_delay_seconds(completed_retries: u32) -> i64 {
    match completed_retries {
        0 => 2,
        1 => 5,
        _ => 10,
    }
}

fn schedule_next_validation_retry(recovery: &mut project::RecoveryState) {
    recovery.next_validation_retry_at = Some(
        (chrono::Utc::now()
            + chrono::Duration::seconds(validation_retry_delay_seconds(
                recovery.validation_retry_count,
            )))
        .to_rfc3339(),
    );
}

pub(crate) fn is_review_validation_recovery(recovery: &project::RecoveryState) -> bool {
    matches!(
        recovery.error_kind,
        project::RecoveryErrorKind::ReviewTransientFailure
            | project::RecoveryErrorKind::ReviewProtocolFailure
    ) && recovery.phase == project::RecoveryPhase::Retesting
}

pub(crate) fn validation_retry_due(recovery: &project::RecoveryState) -> bool {
    recovery
        .next_validation_retry_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_none_or(|deadline| deadline <= chrono::Utc::now())
}

pub(crate) fn validation_retry_can_resume(recovery: &project::RecoveryState) -> bool {
    is_review_validation_recovery(recovery)
        && recovery.validation_retry_count < recovery.max_validation_retries
}

fn record_review_protocol_strategies(
    recovery: &mut project::RecoveryState,
    test: &project::TestResult,
) {
    let mut record = |strategy| {
        if !recovery.validation_strategies.contains(&strategy) {
            recovery.validation_strategies.push(strategy);
        }
    };
    if test.review_failure_kind == Some(project::ReviewFailureKind::FieldTypeMismatch) {
        record(project::ValidationRetryStrategy::DeterministicNormalization);
    }
    if test.review_protocol_attempts > 0 {
        record(project::ValidationRetryStrategy::ProtocolRepair);
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...[truncated]", prefix)
    } else {
        prefix
    }
}

fn normalized_signature(kind: &project::RecoveryErrorKind, details: &str) -> String {
    let normalized = details
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    format!("{:?}:{}", kind, truncate_chars(&normalized, 512))
}

fn record_failed_signature(
    recovery: &mut project::RecoveryState,
    kind: project::RecoveryErrorKind,
    signature: String,
) -> bool {
    if recovery.error_signature == signature {
        recovery.repeated_signature_count = recovery.repeated_signature_count.saturating_add(1);
    } else {
        recovery.repeated_signature_count = 1;
    }
    recovery.error_kind = kind;
    recovery.error_signature = signature;
    recovery.attempt >= recovery.max_attempts || recovery.repeated_signature_count >= 3
}

fn append_failure_history(recovery: &mut project::RecoveryState, failure: &str) {
    let failure = truncate_chars(failure, 4_000);
    if failure.is_empty() || recovery.failure_history.last() == Some(&failure) {
        return;
    }
    recovery.failure_history.push(failure);
    if recovery.failure_history.len() > MAX_FAILURE_HISTORY {
        recovery
            .failure_history
            .drain(0..recovery.failure_history.len() - MAX_FAILURE_HISTORY);
    }
}

fn normalize_issue_component(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn actionable_recovery_issues(
    test: &project::TestResult,
    subtask: &project::Subtask,
    authorized_paths: &[String],
) -> Vec<project::RecoveryIssue> {
    let authorized: BTreeSet<&str> = authorized_paths.iter().map(String::as_str).collect();
    let mut issues = BTreeMap::new();
    for issue in &test.review_issues {
        let Some(criterion_index) = issue.criterion_index else {
            continue;
        };
        if criterion_index == 0
            || criterion_index as usize > subtask.acceptance_criteria.len()
            || !authorized.contains(issue.file.as_str())
            || issue.expected.trim().is_empty()
            || issue.actual.trim().is_empty()
            || issue.suggested_change.trim().is_empty()
            || issue.confidence < 0.7
            || issue.severity != Some(project::ReviewIssueSeverity::Blocking)
            || issue.evidence_references.is_empty()
            || issue.evidence_references.iter().any(|reference| {
                reference.block_id.trim().is_empty()
                    || !authorized.contains(reference.file.as_str())
            })
        {
            continue;
        }
        let criterion = subtask.acceptance_criteria[criterion_index as usize - 1].clone();
        let id = format!(
            "criterion:{}:file:{}",
            criterion_index,
            normalize_issue_component(&issue.file),
        );
        issues.insert(
            id.clone(),
            project::RecoveryIssue {
                id,
                criterion_index: Some(criterion_index),
                criterion,
                file: issue.file.clone(),
                expected: issue.expected.clone(),
                actual: issue.actual.clone(),
                suggested_change: issue.suggested_change.clone(),
                confidence: issue.confidence,
                severity: issue.severity.clone(),
                evidence_references: issue.evidence_references.clone(),
            },
        );
    }
    issues.into_values().collect()
}

fn recovery_issues(
    test: &project::TestResult,
    subtask: &project::Subtask,
    authorized_paths: &[String],
) -> Vec<project::RecoveryIssue> {
    let actionable = actionable_recovery_issues(test, subtask, authorized_paths);
    if !actionable.is_empty() {
        return actionable;
    }
    test.issues
        .iter()
        .filter(|issue| !issue.trim().is_empty())
        .map(|issue| project::RecoveryIssue {
            id: format!(
                "unstructured:{}",
                truncate_chars(&normalize_issue_component(issue), 256)
            ),
            actual: issue.clone(),
            suggested_change: test.suggestion.clone(),
            ..Default::default()
        })
        .collect()
}

fn pending_evidence_criteria(ledger: &[project::AcceptanceLedgerItem]) -> Vec<u32> {
    ledger
        .iter()
        .filter(|item| item.status == project::AcceptanceStatus::Unknown)
        .map(|item| item.criterion_index)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn next_evidence_strategy(
    recovery: &project::RecoveryState,
) -> Option<project::ReviewEvidenceStrategy> {
    match recovery.evidence_rebuild_attempts {
        0 => Some(project::ReviewEvidenceStrategy::Targeted),
        1 => Some(project::ReviewEvidenceStrategy::ExpandedTargeted),
        _ => None,
    }
}

fn merge_targeted_review(
    previous: Option<&project::TestResult>,
    mut targeted: project::TestResult,
    target_indices: &[u32],
) -> project::TestResult {
    let Some(previous) = previous else {
        return targeted;
    };
    let targets = target_indices.iter().copied().collect::<BTreeSet<_>>();
    let mut reviews = previous
        .criterion_reviews
        .iter()
        .filter(|review| !targets.contains(&review.criterion_index))
        .map(|review| (review.criterion_index, review.clone()))
        .collect::<BTreeMap<_, _>>();
    reviews.extend(
        targeted
            .criterion_reviews
            .drain(..)
            .map(|review| (review.criterion_index, review)),
    );
    targeted.criterion_reviews = reviews.into_values().collect();

    let mut issues = previous
        .review_issues
        .iter()
        .filter(|issue| {
            issue
                .criterion_index
                .is_none_or(|index| !targets.contains(&index))
        })
        .cloned()
        .collect::<Vec<_>>();
    issues.append(&mut targeted.review_issues);
    targeted.review_issues = issues;

    targeted.warnings = previous
        .warnings
        .iter()
        .chain(targeted.warnings.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    targeted.review_evidence_summary = format!(
        "原审查：{}；定向补证：{}",
        previous.review_evidence_summary, targeted.review_evidence_summary
    );
    targeted.acceptance_results.clear();
    targeted
}

fn issue_list_for_prompt(issues: &[project::RecoveryIssue]) -> String {
    if issues.is_empty() {
        return "（没有可靠的结构化问题，按失败证据处理）".to_string();
    }
    issues
        .iter()
        .map(|issue| {
            format!(
                "- [{}] 验收项={} 文件={}；预期={}；实际={}；修复目标={}",
                issue.id,
                issue
                    .criterion_index
                    .map(|index| index.to_string())
                    .unwrap_or_else(|| "未关联".to_string()),
                if issue.file.is_empty() {
                    "未关联"
                } else {
                    &issue.file
                },
                if issue.expected.is_empty() {
                    "见失败证据"
                } else {
                    &issue.expected
                },
                issue.actual,
                if issue.suggested_change.is_empty() {
                    "见总体建议"
                } else {
                    &issue.suggested_change
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn attempt_history_for_prompt(history: &[project::RecoveryAttemptRecord]) -> String {
    if history.is_empty() {
        return "（尚无修复轮次）".to_string();
    }
    history
        .iter()
        .map(|record| {
            format!(
                "- 第 {} 轮：解决 {} 项，剩余 {} 项，新增 {} 项，进展={}，变更文件={}；{}",
                record.attempt,
                record.resolved_issue_ids.len(),
                record.remaining_issue_ids.len(),
                record.regressed_issue_ids.len(),
                if record.made_progress { "是" } else { "否" },
                if record.changed_files.is_empty() {
                    "无".to_string()
                } else {
                    record.changed_files.join("、")
                },
                record.summary,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn touch(proj: &mut project::Project) {
    proj.workflow_state.data_revision = proj.workflow_state.data_revision.saturating_add(1);
    proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
}

fn test_failure_summary(test: Option<&project::TestResult>, fallback: &str) -> String {
    let Some(test) = test else {
        return fallback.to_string();
    };
    let mut parts = Vec::new();
    if !test.test_command.is_empty() {
        parts.push(format!("command={}", test.test_command));
    }
    if let Some(code) = test.test_exit_code {
        parts.push(format!("exit_code={}", code));
    }
    if !test.issues.is_empty() {
        parts.push(format!("issues={}", test.issues.join(" | ")));
    }
    if !test.suggestion.is_empty() {
        parts.push(format!("suggestion={}", test.suggestion));
    }
    if !test.test_output_summary.is_empty() {
        parts.push(format!(
            "output={}",
            truncate_chars(&test.test_output_summary, 2_000)
        ));
    }
    if test.review_evidence_status != project::ReviewEvidenceStatus::Complete
        && !test.review_evidence_summary.is_empty()
    {
        parts.push(format!(
            "review_evidence={}",
            truncate_chars(&test.review_evidence_summary, 2_000)
        ));
    }
    if !test.review_diagnostic_summary.is_empty() {
        parts.push(format!(
            "review_diagnostic={}",
            truncate_chars(&test.review_diagnostic_summary, 1_000)
        ));
    }
    if parts.is_empty() {
        fallback.to_string()
    } else {
        parts.join("\n")
    }
}

pub(crate) fn classify_test_result_with_context(
    test: Option<&project::TestResult>,
    subtask: Option<&project::Subtask>,
    authorized_paths: &[String],
) -> project::RecoveryErrorKind {
    let Some(test) = test else {
        return project::RecoveryErrorKind::TestUnavailable;
    };
    let (criterion_count, ledger) = if let Some(subtask) = subtask {
        let ledger = if !test.acceptance_results.is_empty() {
            test.acceptance_results.clone()
        } else if !subtask.acceptance_ledger.is_empty() {
            subtask.acceptance_ledger.clone()
        } else {
            crate::acceptance::build_ledger(&subtask.acceptance_criteria, test, authorized_paths)
        };
        (subtask.acceptance_criteria.len(), ledger)
    } else {
        (
            test.acceptance_results.len(),
            test.acceptance_results.clone(),
        )
    };
    crate::quality_gate::evaluate(Some(test), &ledger, criterion_count, false)
        .recovery_error_kind(Some(test))
}

/// 没有任务契约时，部分审查证据不会被误判为可执行。
#[cfg(test)]
pub(crate) fn classify_test_result(
    test: Option<&project::TestResult>,
) -> project::RecoveryErrorKind {
    classify_test_result_with_context(test, None, &[])
}

fn create_recovery_state(
    kind: project::RecoveryErrorKind,
    subtask_id: String,
    execution_id: String,
    baseline_commit: String,
    failure: String,
) -> project::RecoveryState {
    let now = chrono::Utc::now().to_rfc3339();
    let initial_failure = truncate_chars(&failure, 4_000);
    project::RecoveryState {
        error_signature: normalized_signature(&kind, &initial_failure),
        error_kind: kind,
        phase: project::RecoveryPhase::Diagnosing,
        attempt: 0,
        max_attempts: DEFAULT_MAX_ATTEMPTS,
        repeated_signature_count: 1,
        subtask_id,
        execution_id,
        baseline_commit,
        last_diagnosis: String::new(),
        last_repair_summary: String::new(),
        original_test_failure: initial_failure.clone(),
        replan_attempted: false,
        failure_history: if initial_failure.is_empty() {
            vec![]
        } else {
            vec![initial_failure]
        },
        active_issues: vec![],
        attempt_history: vec![],
        replan_execution_attempted: false,
        started_at: now.clone(),
        updated_at: now,
        engine_failure_kind: None,
        checkpoint_id: String::new(),
        rollback_retest_pending: false,
        evidence_rebuild_attempted: false,
        evidence_rebuild_attempts: 0,
        pending_evidence_criteria: vec![],
        evidence_strategies: vec![],
        validation_retry_count: 0,
        max_validation_retries: 3,
        next_validation_retry_at: None,
        validation_strategies: vec![],
        pending_execution_result: None,
    }
}

fn set_autopilot_recovering(proj: &mut project::Project, description: &str) {
    if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
        autopilot.run_status = project::AutopilotRunStatus::Running;
        autopilot.last_action = description.to_string();
        autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
        autopilot.error_message.clear();
        autopilot.recovery_action = project::AutopilotRecoveryAction::RunAutomaticRecovery;
    }
}

fn set_autopilot_waiting(proj: &mut project::Project, description: &str) {
    if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
        autopilot.run_status = project::AutopilotRunStatus::ErrorStopped;
        autopilot.last_action = description.to_string();
        autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
        autopilot.error_message = description.to_string();
        autopilot.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
    }
}

pub(crate) fn begin_execution_recovery(
    proj: &mut project::Project,
    kind: project::RecoveryErrorKind,
    execution_id: &str,
    failure: &str,
) {
    if !proj.workflow_state.autopilot_active {
        return;
    }
    let Some(session) = proj.execution_session.as_ref() else {
        return;
    };
    let state = create_recovery_state(
        kind.clone(),
        session.subtask_id.clone(),
        execution_id.to_string(),
        session.base_commit.clone(),
        truncate_chars(failure, 4_000),
    );
    proj.workflow_state.recovery_state = Some(state);
    write_recovery_history(
        proj,
        "error",
        project::ExecutionEventType::RecoveryStarted,
        format!("错误恢复已启动：{:?}", kind),
        Some(&session.milestone_id.clone()),
        Some(&session.mid_stage_id.clone()),
        Some(&session.subtask_id.clone()),
    );
    set_autopilot_recovering(proj, "正在诊断执行错误");
    touch(proj);
}

pub(crate) fn ensure_quality_recovery(
    proj: &mut project::Project,
    gate_reason: &str,
) -> Result<bool, String> {
    let session = proj
        .execution_session
        .as_ref()
        .ok_or_else(|| "质量门禁失败但没有执行会话。".to_string())?
        .clone();
    let subtask = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter()
                .find(|item| item.id == session.subtask_id)
        })
        .ok_or_else(|| "质量门禁失败但无法定位当前小阶段。".to_string())?;

    if let Some(recovery) = proj.workflow_state.recovery_state.as_ref() {
        if recovery.subtask_id == session.subtask_id
            && !matches!(recovery.phase, project::RecoveryPhase::Recovered)
        {
            return Ok(matches!(
                recovery.phase,
                project::RecoveryPhase::Diagnosing
                    | project::RecoveryPhase::Repairing
                    | project::RecoveryPhase::Retesting
                    | project::RecoveryPhase::Replanning
            ));
        }
    }

    let kind = if subtask
        .execution_result
        .as_ref()
        .is_none_or(|result| !result.success)
    {
        project::RecoveryErrorKind::ExecutionError
    } else {
        let authorized_paths = crate::plan_contract::validate_subtask(subtask, "质量恢复任务")?;
        classify_test_result_with_context(
            subtask.test_result.as_ref(),
            Some(subtask),
            &authorized_paths,
        )
    };
    let failure = test_failure_summary(subtask.test_result.as_ref(), gate_reason);
    let mut recovery = create_recovery_state(
        kind.clone(),
        subtask.id.clone(),
        session.execution_id.clone(),
        session.base_commit.clone(),
        truncate_chars(&failure, 4_000),
    );
    let authorized_paths = crate::plan_contract::validate_subtask(subtask, "质量恢复任务")?;
    if let Some(test) = subtask.test_result.as_ref() {
        recovery.active_issues = recovery_issues(test, subtask, &authorized_paths);
    }
    if kind == project::RecoveryErrorKind::EvidenceInsufficient {
        let ledger = if subtask.acceptance_ledger.is_empty() {
            subtask
                .test_result
                .as_ref()
                .map(|test| {
                    crate::acceptance::build_ledger(
                        &subtask.acceptance_criteria,
                        test,
                        &authorized_paths,
                    )
                })
                .unwrap_or_default()
        } else {
            subtask.acceptance_ledger.clone()
        };
        recovery.pending_evidence_criteria = pending_evidence_criteria(&ledger);
        recovery.active_issues.clear();
    }
    if let Some(limit) = validation_retry_limit(&kind) {
        recovery.phase = project::RecoveryPhase::Retesting;
        recovery.max_validation_retries = limit;
        recovery.active_issues.clear();
        if kind == project::RecoveryErrorKind::ReviewProtocolFailure {
            if let Some(test) = subtask.test_result.as_ref() {
                record_review_protocol_strategies(&mut recovery, test);
            }
        }
        schedule_next_validation_retry(&mut recovery);
    }
    let automatic = !matches!(
        kind,
        project::RecoveryErrorKind::TestUnavailable
            | project::RecoveryErrorKind::AutomatedTestUnavailable
            | project::RecoveryErrorKind::ReviewServiceBlocked
            | project::RecoveryErrorKind::ContractContradiction
            | project::RecoveryErrorKind::ValidationOscillation
    );
    if matches!(
        kind,
        project::RecoveryErrorKind::EvidenceInsufficient
            | project::RecoveryErrorKind::ValidationFailure
    ) {
        recovery.phase = project::RecoveryPhase::Retesting;
    } else if kind == project::RecoveryErrorKind::PlanFailure {
        recovery.phase = project::RecoveryPhase::Replanning;
    } else if !automatic {
        recovery.phase = project::RecoveryPhase::WaitingHuman;
    }
    proj.workflow_state.recovery_state = Some(recovery);
    write_recovery_history(
        proj,
        "error",
        project::ExecutionEventType::RecoveryStarted,
        format!("质量错误已分类：{:?}", kind),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    let validation_audit = proj.workflow_state.recovery_state.as_ref().map(|state| {
        (
            state.validation_strategies.clone(),
            state.validation_retry_count,
            state.max_validation_retries,
            state.next_validation_retry_at.clone(),
        )
    });
    if let Some((strategies, completed, limit, retry_at)) = validation_audit {
        if strategies.contains(&project::ValidationRetryStrategy::DeterministicNormalization) {
            write_recovery_history(
                proj,
                "info",
                project::ExecutionEventType::ProtocolNormalized,
                "审查协议已执行确定性归一化，未修改代码或执行基线".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
        }
        if strategies.contains(&project::ValidationRetryStrategy::ProtocolRepair) {
            write_recovery_history(
                proj,
                "info",
                project::ExecutionEventType::ProtocolRepairAttempted,
                "审查协议已执行一次带 Schema 的格式修复".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
        }
        if let Some(retry_at) = retry_at {
            write_recovery_history(
                proj,
                "info",
                project::ExecutionEventType::ValidationRetryScheduled,
                format!(
                    "验证重试已安排：第 {}/{} 次，最早执行时间 {}",
                    completed.saturating_add(1),
                    limit,
                    retry_at
                ),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
        }
    }
    if automatic {
        set_autopilot_recovering(
            proj,
            if matches!(
                kind,
                project::RecoveryErrorKind::EvidenceInsufficient
                    | project::RecoveryErrorKind::ValidationFailure
                    | project::RecoveryErrorKind::ReviewTransientFailure
                    | project::RecoveryErrorKind::ReviewProtocolFailure
            ) {
                if matches!(
                    kind,
                    project::RecoveryErrorKind::ReviewTransientFailure
                        | project::RecoveryErrorKind::ReviewProtocolFailure
                ) {
                    "正在等待重新请求 AI 审查"
                } else {
                    "正在重建验收证据"
                }
            } else if kind == project::RecoveryErrorKind::PlanFailure {
                "当前任务契约与项目事实不一致，正在受限重规划"
            } else {
                "正在诊断质量错误"
            },
        );
    } else {
        let message = match kind {
            project::RecoveryErrorKind::ReviewServiceBlocked => {
                "AI 审查认证或额度异常，需要人工处理后重新验证"
            }
            project::RecoveryErrorKind::AutomatedTestUnavailable => {
                "自动化测试环境不可用，需要人工恢复测试环境"
            }
            _ => "验收证据不可用或不足，需要重建证据后再判断",
        };
        set_autopilot_waiting(proj, message);
        if let Some(current) = proj.execution_session.as_mut() {
            current.status = "quality_blocked".to_string();
            current.failure_message = gate_reason.to_string();
        }
    }
    touch(proj);
    Ok(automatic)
}

pub(crate) fn begin_rejected_recovery(
    proj: &mut project::Project,
    reason: &str,
) -> Result<(), String> {
    if !proj.workflow_state.autopilot_active {
        return Ok(());
    }
    let session = proj
        .execution_session
        .as_ref()
        .ok_or_else(|| "驳回结果缺少执行会话。".to_string())?
        .clone();
    let mut recovery = create_recovery_state(
        project::RecoveryErrorKind::ReviewFailure,
        session.subtask_id.clone(),
        session.execution_id.clone(),
        session.base_commit.clone(),
        truncate_chars(reason, 4_000),
    );
    recovery.original_test_failure = format!("人工驳回：{}", truncate_chars(reason, 3_000));
    proj.workflow_state.recovery_state = Some(recovery);
    if let Some(current_session) = proj.execution_session.as_mut() {
        current_session.active = true;
        current_session.status = "quality_blocked".to_string();
        current_session.failure_message = reason.to_string();
        current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
    }
    write_recovery_history(
        proj,
        "error",
        project::ExecutionEventType::RecoveryStarted,
        "人工驳回已进入受限修复循环".to_string(),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    set_autopilot_recovering(proj, "正在诊断人工驳回的问题");
    touch(proj);
    Ok(())
}

fn current_recovery_context(
    proj: &project::Project,
) -> Result<
    (
        project::RecoveryState,
        project::ExecutionSession,
        project::Subtask,
    ),
    String,
> {
    let recovery = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .ok_or_else(|| "当前没有错误恢复任务。".to_string())?
        .clone();
    let session = proj
        .execution_session
        .as_ref()
        .ok_or_else(|| "恢复任务缺少执行会话。".to_string())?
        .clone();
    if session.subtask_id != recovery.subtask_id {
        return Err("恢复任务与执行会话不一致。".to_string());
    }
    let subtask = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter()
                .find(|item| item.id == session.subtask_id)
        })
        .ok_or_else(|| "无法定位恢复任务对应的小阶段。".to_string())?
        .clone();
    Ok((recovery, session, subtask))
}

fn execution_snapshot_mismatch(
    session: &project::ExecutionSession,
    settings: &crate::settings::AppSettings,
    health: &crate::engine::EngineHealth,
) -> Option<String> {
    if session.engine_settings_revision == 0 {
        let defaults = crate::settings::AppSettings::default();
        if settings.decision_model != defaults.decision_model
            || settings.built_in_grok_build != defaults.built_in_grok_build
            || settings.plugin_cli != defaults.plugin_cli
        {
            return Some("旧执行会话没有设置修订号，且当前应用设置不是兼容默认值".to_string());
        }
    } else if session.engine_settings_revision != settings.revision {
        return Some(format!(
            "应用设置修订已变化（会话 {}，当前 {}）",
            session.engine_settings_revision, settings.revision
        ));
    }

    if session.engine_snapshot.runtime == project::ExecutionRuntime::BuiltIn {
        let current_source = health.source_revision.as_deref().unwrap_or_default();
        if session.engine_source_revision.is_empty() {
            return Some("旧内置执行会话没有 Grok Build 源码修订，必须重新确认".to_string());
        }
        if session.engine_source_revision != current_source {
            return Some(format!(
                "Grok Build 源码修订已变化（会话 {}，当前 {}）",
                session.engine_source_revision, current_source
            ));
        }
        let current_backend = settings.built_in_grok_build.api_backend.as_str();
        if session.engine_api_backend.is_empty() {
            return Some("旧内置执行会话没有 API 后端快照，必须重新确认".to_string());
        }
        if session.engine_api_backend != current_backend {
            return Some(format!(
                "Grok Build API 后端已变化（会话 {}，当前 {}）",
                session.engine_api_backend, current_backend
            ));
        }
        let fingerprint =
            crate::settings::endpoint_fingerprint(&settings.built_in_grok_build.api_base_url);
        if session.engine_model.is_empty() {
            return Some("旧内置执行会话没有模型快照，必须重新确认".to_string());
        }
        if session.engine_model != settings.built_in_grok_build.model {
            return Some("预装引擎模型与执行快照不一致".to_string());
        }
        if session.endpoint_fingerprint.is_empty() {
            return Some("旧内置执行会话没有接口地址快照，必须重新确认".to_string());
        }
        if session.endpoint_fingerprint != fingerprint {
            return Some("预装引擎接口地址与执行快照不一致".to_string());
        }
    }

    if session.engine_snapshot.runtime == project::ExecutionRuntime::Plugin
        && !session.engine_executable_path.is_empty()
        && health.executable_path.as_deref() != Some(session.engine_executable_path.as_str())
    {
        return Some("插件可执行文件路径与执行快照不一致".to_string());
    }
    None
}

fn wait_for_engine_snapshot_confirmation(
    proj: &mut project::Project,
    recovery: &mut project::RecoveryState,
    session: &project::ExecutionSession,
    message: &str,
) {
    recovery.attempt = recovery.attempt.saturating_sub(1);
    recovery.error_kind = project::RecoveryErrorKind::EngineBlocked;
    recovery.phase = project::RecoveryPhase::WaitingEngine;
    recovery.last_repair_summary = truncate_chars(message, 4_000);
    recovery.updated_at = chrono::Utc::now().to_rfc3339();
    proj.workflow_state.recovery_state = Some(recovery.clone());
    preserve_recovery_session(proj, session, &recovery.execution_id);
    if let Some(current_session) = proj.execution_session.as_mut() {
        current_session.failure_message = truncate_chars(message, 2_048);
    }
    set_autopilot_waiting(proj, message);
    write_recovery_history(
        proj,
        "error",
        project::ExecutionEventType::ExecutionFailed,
        message.to_string(),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    touch(proj);
}

fn git_diff_evidence(project_path: &str, allowed_paths: &[String]) -> String {
    let mut command = std::process::Command::new("git");
    command.args(["diff", "--no-ext-diff", "--unified=3", "--"]);
    for path in allowed_paths {
        command.arg(path);
    }
    match command.current_dir(project_path).output() {
        Ok(output) if output.status.success() => {
            truncate_chars(&String::from_utf8_lossy(&output.stdout), MAX_EVIDENCE_CHARS)
        }
        Ok(output) => format!(
            "读取 diff 失败：{}",
            truncate_chars(&String::from_utf8_lossy(&output.stderr), 1_000)
        ),
        Err(error) => format!("读取 diff 失败：{}", error),
    }
}

fn build_diagnosis(
    proj: &project::Project,
    recovery: &project::RecoveryState,
    subtask: &project::Subtask,
    authorized_paths: &[String],
) -> String {
    let current_test = test_failure_summary(
        subtask.test_result.as_ref(),
        &recovery.original_test_failure,
    );
    let test = if recovery.original_test_failure.is_empty()
        || current_test == recovery.original_test_failure
    {
        current_test
    } else {
        format!(
            "原始失败：\n{}\n\n当前测试证据：\n{}",
            recovery.original_test_failure, current_test
        )
    };
    let execution_error = subtask
        .execution_result
        .as_ref()
        .map(|result| truncate_chars(&result.error_log, 2_000))
        .unwrap_or_default();
    let diff = git_diff_evidence(&proj.project_path, authorized_paths);
    let strategy_note = recovery
        .attempt_history
        .last()
        .filter(|record| !record.made_progress)
        .map(|_| "\n策略要求：上一轮没有取得可验证进展，本轮必须更换实现策略，不得重复同一修改。")
        .unwrap_or_default();
    let learning = crate::recovery_learning::render_matching(
        proj,
        subtask,
        Some(&format!("{:?}", recovery.error_kind)),
        Some(&recovery.error_signature),
    );
    truncate_chars(
        &format!(
            "恢复类型：{:?}\n当前目标：{}\n验收标准（最高优先级，精确标识符必须逐字遵循）：\n- {}\n当前未满足项：\n{}\n修复轮次历史：\n{}{}\n匹配的纠错经验：\n{}\n允许修改：\n- {}\n允许新建：\n- {}\n当前基线：{}\n失败证据：\n{}\n执行错误：\n{}\n当前受限 diff：\n{}\n上次修复摘要：\n{}",
            recovery.error_kind,
            if subtask.goal.is_empty() {
                &subtask.title
            } else {
                &subtask.goal
            },
            subtask.acceptance_criteria.join("\n- "),
            issue_list_for_prompt(&recovery.active_issues),
            attempt_history_for_prompt(&recovery.attempt_history),
            strategy_note,
            if learning.is_empty() {
                "（无）"
            } else {
                &learning
            },
            authorized_paths.join("\n- "),
            subtask.new_file_paths.join("\n- "),
            recovery.baseline_commit,
            test,
            execution_error,
            diff,
            recovery.last_repair_summary,
        ),
        MAX_DIAGNOSIS_CHARS,
    )
}

fn repair_prompt(
    recovery: &project::RecoveryState,
    subtask: &project::Subtask,
    diagnosis: &str,
) -> String {
    let original = crate::plan_compiler::compile_execution_prompt(subtask);
    if recovery.replan_attempted {
        format!(
            "执行受限重规划后的当前小阶段完整任务。工作区已恢复到该任务开始前的 Git 基线；必须完整实现全部验收标准，不得只补最后一次差异，不得扩大任务范围。\n\n重规划后的完整任务：\n{}\n\n失败历史与安全边界：\n{}",
            original, diagnosis
        )
    } else if recovery.error_kind == project::RecoveryErrorKind::ExecutionError {
        format!(
            "重新执行已批准的当前小阶段。上次执行器异常，已恢复到执行基线。不得扩大任务范围。\n\n原始任务：\n{}\n\n异常摘要：\n{}",
            original, diagnosis
        )
    } else {
        format!(
            "只修复当前小阶段的已知失败，不重新设计、不扩展任务范围。验收标准高于原始执行提示；验收标准中的函数名、字段名、API 名和行为必须精确匹配。保留已经满足的验收项，只处理当前未满足项。完成修复后直接结束。\n\n原始任务：\n{}\n\n受限诊断上下文：\n{}",
            original, diagnosis
        )
    }
}

fn validate_replan_output(
    output: crate::plan_calibration::PlanPatchOutput,
) -> Result<crate::plan_calibration::PlanPatchOutput, String> {
    if output.implementation_guidance.trim().is_empty()
        || output.context_summary.trim().is_empty()
        || output.evidence_files.is_empty()
        || output.dependency_notes.trim().is_empty()
    {
        return Err("当前任务计划补丁缺少实现指引、当前背景、证据文件或依赖说明。".to_string());
    }
    Ok(output)
}

async fn replan_current_subtask(
    proj: &mut project::Project,
    recovery: &project::RecoveryState,
    session: &project::ExecutionSession,
    subtask: &project::Subtask,
    authorized_paths: &[String],
) -> Result<(), String> {
    if recovery.replan_attempted {
        return Err("当前小阶段已经执行过受限重规划。".to_string());
    }
    if subtask.acceptance_criteria.is_empty() {
        return Err("当前小阶段没有可供重规划核对的验收标准。".to_string());
    }
    let frozen_diff = git_diff_evidence(&proj.project_path, authorized_paths);
    let criteria = subtask
        .acceptance_criteria
        .iter()
        .enumerate()
        .map(|(index, criterion)| format!("{}. {}", index + 1, criterion))
        .collect::<Vec<_>>()
        .join("\n");
    let failure_history = if recovery.failure_history.is_empty() {
        recovery.original_test_failure.clone()
    } else {
        recovery
            .failure_history
            .iter()
            .enumerate()
            .map(|(index, failure)| format!("第 {} 轮：{}", index + 1, failure))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let context = truncate_chars(
        &format!(
            "当前小阶段：{}\n目标：{}\n\n原始执行契约（保持原意）：\n{}\n\n当前执行提示（仅供识别旧计划缺陷）：\n{}\n\n不可变验收标准：\n{}\n\n允许修改：\n- {}\n允许新建：\n- {}\n停止规则：\n- {}\n\n当前未满足项：\n{}\n\n失败历史：\n{}\n\n恢复前受限 diff（重执行时不会保留）：\n{}",
            subtask.title,
            if subtask.goal.is_empty() {
                &subtask.title
            } else {
                &subtask.goal
            },
            subtask.prompt,
            subtask.execution_prompt,
            criteria,
            authorized_paths.join("\n- "),
            subtask.new_file_paths.join("\n- "),
            subtask.stop_rules.join("\n- "),
            issue_list_for_prompt(&recovery.active_issues),
            failure_history,
            frozen_diff,
        ),
        MAX_DIAGNOSIS_CHARS,
    );

    let target = if recovery.baseline_commit.is_empty() {
        "HEAD"
    } else {
        &recovery.baseline_commit
    };
    pipeline::restore_git_execution_baseline(&proj.project_path, target)
        .map_err(|error| format!("当前任务重规划前恢复执行基线失败：{}", error))?;

    let current_facts = crate::project_facts::capture_with_identifiers(
        &proj.project_path,
        &crate::project_facts::snapshot_paths(subtask),
        crate::project_facts::accepted_deviations(proj),
        &subtask.required_identifiers,
    )?;
    let current_facts_text = serde_json::to_string_pretty(&current_facts)
        .map_err(|error| format!("序列化当前任务事实失败：{}", error))?;
    let context = truncate_chars(
        &format!(
            "恢复基线上的最新项目事实：\n{}\n\n{}",
            current_facts_text, context
        ),
        MAX_DIAGNOSIS_CHARS,
    );

    let reply =
        crate::api::call_deepseek_api_json(crate::prompts::RECOVERY_REPLAN_PROMPT, &context)
            .await
            .map_err(|error| format!("当前任务重规划 AI 调用失败：{}", error))?;
    let output: crate::plan_calibration::PlanPatchOutput =
        crate::json_utils::parse_json_with_retry(&reply)
            .await
            .map_err(|error| format!("当前任务重规划结果解析失败：{}", error))?;
    let output = validate_replan_output(output)?;
    let contract_before = crate::plan_calibration::immutable_contract(subtask)?;
    let mut contract_candidate = subtask.clone();
    contract_candidate.execution_prompt = output.implementation_guidance.trim().to_string();
    contract_candidate.context_summary = output.context_summary.trim().to_string();
    contract_candidate.evidence_files = output.evidence_files.clone();
    contract_candidate.dependency_notes = output.dependency_notes.trim().to_string();
    crate::plan_contract::hydrate_subtask_contract(&mut contract_candidate);
    if crate::plan_calibration::immutable_contract(&contract_candidate)? != contract_before {
        return Err("当前任务计划补丁改变了不可变契约，已拒绝。".to_string());
    }
    crate::plan_contract::validate_subtask(&contract_candidate, "当前小阶段重规划")?;
    crate::plan_contract::validate_execution_prompt(&contract_candidate, "当前小阶段重规划")?;

    let item = proj
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter_mut()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter_mut()
                .find(|item| item.id == session.subtask_id)
        })
        .ok_or_else(|| "重规划完成后无法定位当前小阶段。".to_string())?;
    item.execution_prompt = contract_candidate.execution_prompt;
    item.context_summary = contract_candidate.context_summary;
    item.evidence_files = contract_candidate.evidence_files;
    item.dependency_notes = contract_candidate.dependency_notes;
    item.required_identifiers = contract_candidate.required_identifiers;
    item.fact_snapshot = Some(current_facts);
    item.plan_patch_revision = item.plan_patch_revision.saturating_add(1);
    item.status = project::SubtaskStatus::Pending;
    item.execution_result = None;
    item.test_result = None;
    item.human_verification = None;

    let now = chrono::Utc::now().to_rfc3339();
    let current = proj
        .workflow_state
        .recovery_state
        .as_mut()
        .ok_or_else(|| "重规划完成时恢复状态已丢失。".to_string())?;
    current.phase = project::RecoveryPhase::Diagnosing;
    current.attempt = 0;
    current.repeated_signature_count = 1;
    current.replan_attempted = true;
    current.replan_execution_attempted = false;
    current.last_repair_summary = if output.rationale.trim().is_empty() {
        "当前小阶段已受限重规划，准备从基线完整重执行".to_string()
    } else {
        format!("当前小阶段已受限重规划：{}", output.rationale.trim())
    };
    current.updated_at = now.clone();
    if let Some(current_session) = proj.execution_session.as_mut() {
        current_session.active = false;
        current_session.status = "replan_ready".to_string();
        current_session.failure_message.clear();
        current_session.state_entered_at = now;
    }
    write_recovery_history(
        proj,
        "success",
        project::ExecutionEventType::ReplanCompleted,
        "当前小阶段受限重规划完成，准备从执行基线完整重执行".to_string(),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    set_autopilot_recovering(proj, "当前任务已重规划，准备从基线重新执行");
    touch(proj);
    Ok(())
}

fn set_subtask_running(proj: &mut project::Project, session: &project::ExecutionSession) {
    if let Some(subtask) = proj
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter_mut()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter_mut()
                .find(|item| item.id == session.subtask_id)
        })
    {
        subtask.status = project::SubtaskStatus::Executing;
    }
}

fn reset_subtask_to_pending(proj: &mut project::Project, session: &project::ExecutionSession) {
    if let Some(subtask) = proj
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter_mut()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter_mut()
                .find(|item| item.id == session.subtask_id)
        })
    {
        subtask.status = project::SubtaskStatus::Pending;
        subtask.execution_result = None;
        subtask.test_result = None;
        subtask.human_verification = None;
    }
}

fn finish_repair_checkpoint(proj: &mut project::Project, restore: bool) -> Result<(), String> {
    let checkpoint_id = proj
        .workflow_state
        .recovery_state
        .as_mut()
        .map(|state| std::mem::take(&mut state.checkpoint_id))
        .unwrap_or_default();
    if checkpoint_id.is_empty() {
        return Ok(());
    }
    if restore {
        crate::recovery_checkpoint::restore(&checkpoint_id)
    } else {
        crate::recovery_checkpoint::discard(&checkpoint_id)
    }
}

fn preserve_recovery_session(
    proj: &mut project::Project,
    session: &project::ExecutionSession,
    execution_id: &str,
) {
    let mut preserved = session.clone();
    preserved.execution_id = execution_id.to_string();
    preserved.active = false;
    preserved.status = "execution_failed".to_string();
    preserved.state_entered_at = chrono::Utc::now().to_rfc3339();
    proj.execution_session = Some(preserved);
}

fn mark_waiting_human(
    proj: &mut project::Project,
    kind: project::RecoveryErrorKind,
    message: &str,
) {
    if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
        recovery.error_kind = kind;
        recovery.phase = project::RecoveryPhase::WaitingHuman;
        recovery.last_repair_summary = truncate_chars(message, 4_000);
        recovery.updated_at = chrono::Utc::now().to_rfc3339();
    }
    if let Some(session) = proj.execution_session.as_mut() {
        session.active = true;
        session.status = "quality_blocked".to_string();
        session.failure_message = truncate_chars(message, 2_048);
        session.state_entered_at = chrono::Utc::now().to_rfc3339();
    }
    set_autopilot_waiting(proj, message);
    touch(proj);
}

fn set_pipeline_terminal(
    pipeline_state: &mut Option<PipelineState>,
    execution_id: &str,
    test: Option<project::TestResult>,
    error: Option<&str>,
) {
    if let Some(pipeline) = pipeline_state.as_mut() {
        if pipeline.execution_id != execution_id {
            return;
        }
        pipeline.status = if error.is_some() {
            PipelineStatus::Failed
        } else {
            PipelineStatus::Paused
        };
        pipeline.awaiting_confirmation = error.is_none();
        pipeline.last_error = error.map(ToString::to_string);
        let current_subtask_id = pipeline.current_subtask_id.clone();
        if let Some(status) = pipeline
            .subtask_statuses
            .iter_mut()
            .find(|status| status.subtask_id == current_subtask_id)
        {
            status.status = if error.is_some() {
                "retrying".to_string()
            } else {
                "testing".to_string()
            };
            status.test_result = test;
        }
    }
}

fn set_pipeline_retesting(pipeline_state: &mut Option<PipelineState>, execution_id: &str) {
    if let Some(pipeline) = pipeline_state.as_mut() {
        if pipeline.execution_id != execution_id {
            return;
        }
        pipeline.status = PipelineStatus::Running;
        pipeline.awaiting_confirmation = false;
        pipeline.last_error = None;
        let current_subtask_id = pipeline.current_subtask_id.clone();
        if let Some(status) = pipeline
            .subtask_statuses
            .iter_mut()
            .find(|status| status.subtask_id == current_subtask_id)
        {
            status.status = "retesting".to_string();
            status.test_result = None;
        }
        pipeline::append_log(
            pipeline,
            "info",
            "正在重新测试恢复后的真实工作区".to_string(),
        );
    }
}

fn set_pipeline_retest_result(
    pipeline_state: &mut Option<PipelineState>,
    execution_id: &str,
    test: project::TestResult,
    proj: &project::Project,
    rolled_back: bool,
) {
    let awaiting_confirmation = proj.workflow_state.recovery_state.is_none()
        && proj.execution_session.as_ref().is_some_and(|session| {
            session.execution_id == execution_id
                && session.status.eq_ignore_ascii_case("awaiting_confirmation")
        });
    if let Some(pipeline) = pipeline_state.as_mut() {
        if pipeline.execution_id != execution_id {
            return;
        }
        pipeline.status = PipelineStatus::Paused;
        pipeline.awaiting_confirmation = awaiting_confirmation;
        pipeline.last_error = None;
        let current_subtask_id = pipeline.current_subtask_id.clone();
        if let Some(status) = pipeline
            .subtask_statuses
            .iter_mut()
            .find(|status| status.subtask_id == current_subtask_id)
        {
            status.status = if awaiting_confirmation {
                "testing"
            } else if rolled_back {
                "retest_pending"
            } else {
                "retrying"
            }
            .to_string();
            status.test_result = (!rolled_back).then_some(test);
        }
    }
}

fn merge_execution_result(
    previous: Option<project::ExecutionResult>,
    repair: project::ExecutionResult,
) -> project::ExecutionResult {
    let mut paths = BTreeSet::new();
    let mut output = String::new();
    if let Some(previous) = previous {
        paths.extend(previous.file_changes);
        output.push_str(&previous.output);
        output.push_str("\n\n=== recovery ===\n");
    }
    paths.extend(repair.file_changes);
    output.push_str(&repair.output);
    project::ExecutionResult {
        success: repair.success,
        output: truncate_chars(&output, 32_000),
        error_log: repair.error_log,
        file_changes: paths.into_iter().collect(),
        exit_code: repair.exit_code,
        engine_provider: repair.engine_provider,
        engine_runtime: repair.engine_runtime,
        engine_settings_revision: repair.engine_settings_revision,
        engine_source_revision: repair.engine_source_revision,
        engine_api_backend: repair.engine_api_backend,
        stdout: repair.stdout,
        stderr: repair.stderr,
        engine_failure_kind: repair.engine_failure_kind,
    }
}

async fn run_recovery_retest(
    pipeline_state: &std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: &str,
    session: &project::ExecutionSession,
    authorized_paths: &[String],
    execution_id: &str,
    evidence_request: Option<crate::test_runner::ReviewEvidenceRequest>,
    retest_kind: RecoveryRetestKind,
) -> Result<project::Project, String> {
    {
        let mut pipeline_guard = pipeline_state.lock().await;
        set_pipeline_retesting(&mut pipeline_guard, execution_id);
    }
    let project = crate::load_project(project_name)?;
    let subtask = project
        .milestones
        .iter()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid| mid.id == session.mid_stage_id)
        })
        .and_then(|mid| {
            mid.subtasks
                .iter()
                .find(|item| item.id == session.subtask_id)
        })
        .ok_or_else(|| "复测时无法定位当前小阶段。".to_string())?
        .clone();
    let prompt = if subtask.execution_prompt.is_empty() {
        subtask.prompt.clone()
    } else {
        subtask.execution_prompt.clone()
    };
    let previous_test = subtask.test_result.clone();
    let evidence_strategy = evidence_request
        .as_ref()
        .map(|request| request.strategy.clone());
    let target_indices = evidence_request
        .as_ref()
        .map(|request| request.target_criterion_indices.clone())
        .unwrap_or_default();
    let goal = if subtask.goal.is_empty() {
        &subtask.title
    } else {
        &subtask.goal
    };
    let progress_project_name = project_name.to_string();
    let progress_execution_id = execution_id.to_string();
    let progress: crate::test_runner::VerificationProgressReporter =
        std::sync::Arc::new(move |stage| {
            let _ = crate::pipeline::persist_verification_progress(
                &progress_project_name,
                &progress_execution_id,
                stage,
            );
        });
    let mut test = match retest_kind {
        RecoveryRetestKind::Full => {
            crate::test_runner::check_subtask_with_context_and_progress(
                &project.project_path,
                goal,
                &session.subtask_id,
                &session.milestone_id,
                &session.mid_stage_id,
                Some(subtask.acceptance_criteria.clone()),
                Some(authorized_paths.to_vec()),
                Some(prompt),
                evidence_request,
                progress.clone(),
            )
            .await
        }
        RecoveryRetestKind::ReviewOnly => {
            let previous = previous_test
                .as_ref()
                .ok_or_else(|| "重新审查缺少可复用的自动化测试结果。".to_string())?;
            crate::test_runner::retry_subtask_review_with_context(
                &project.project_path,
                goal,
                &session.subtask_id,
                &session.milestone_id,
                &session.mid_stage_id,
                Some(subtask.acceptance_criteria.clone()),
                Some(authorized_paths.to_vec()),
                Some(prompt),
                previous,
                progress,
            )
            .await
        }
    }
    .unwrap_or(project::TestResult {
        passed: false,
        issues: vec!["测试服务不可用".to_string()],
        suggestion: "请人工核验".to_string(),
        automated_test_status: project::AutomatedTestStatus::Unavailable,
        ..Default::default()
    });
    if retest_kind == RecoveryRetestKind::ReviewOnly {
        test.review_protocol_attempts = test.review_protocol_attempts.saturating_add(
            previous_test
                .as_ref()
                .map(|previous| previous.review_protocol_attempts)
                .unwrap_or_default(),
        );
    }
    let test = if evidence_strategy.is_some() {
        merge_targeted_review(previous_test.as_ref(), test, &target_indices)
    } else {
        test
    };

    let mut pipeline_guard = pipeline_state.lock().await;
    let mut proj = crate::load_project(project_name)?;
    let still_current = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|current| current.execution_id == execution_id)
        && proj.execution_session.as_ref().is_some_and(|current| {
            current.active
                && current.status.eq_ignore_ascii_case("recovering")
                && current.execution_id == execution_id
        });
    if !still_current {
        return Ok(proj);
    }
    let rolled_back = finish_retest(&mut proj, session, execution_id, test.clone())?;
    if let Some(strategy) = evidence_strategy {
        write_recovery_history(
            &mut proj,
            "info",
            project::ExecutionEventType::EvidenceRebuildCompleted,
            format!("验收证据补充完成：{:?}", strategy),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        if proj
            .workflow_state
            .recovery_state
            .as_ref()
            .is_some_and(|recovery| {
                recovery.error_kind == project::RecoveryErrorKind::EvidenceInsufficient
            })
        {
            write_recovery_history(
                &mut proj,
                "error",
                project::ExecutionEventType::EvidenceStillInsufficient,
                "定向补证后仍有验收项缺少有效证据".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
        }
    }
    set_pipeline_retest_result(&mut pipeline_guard, execution_id, test, &proj, rolled_back);
    crate::save_project(&proj)?;
    crate::load_project(project_name)
}

#[tauri::command]
pub(crate) async fn run_error_recovery(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    run_error_recovery_with_pipeline(state.pipeline_state.clone(), project_name).await
}

pub(crate) async fn run_error_recovery_with_pipeline(
    pipeline_state: std::sync::Arc<tokio::sync::Mutex<Option<PipelineState>>>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut pipeline_guard = pipeline_state.lock().await;
    if pipeline_guard
        .as_ref()
        .is_some_and(|pipeline| pipeline.status == PipelineStatus::Running)
    {
        return Err("已有执行或恢复任务正在运行。".to_string());
    }

    let mut proj = crate::load_project(&project_name)?;
    let (mut recovery, mut session, subtask) = current_recovery_context(&proj)?;
    if recovery.phase == project::RecoveryPhase::WaitingEngine
        || recovery.error_kind == project::RecoveryErrorKind::EngineBlocked
    {
        return Err("执行引擎当前不可用；请恢复额度/认证、切换引擎或稍后重试。".to_string());
    }
    if recovery.phase == project::RecoveryPhase::WaitingHuman {
        return Err("自动恢复已停止，等待人工处理。".to_string());
    }
    let authorized_paths = crate::plan_contract::validate_subtask(&subtask, "错误恢复任务")?;
    if recovery.rollback_retest_pending {
        if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
            current.phase = project::RecoveryPhase::Retesting;
            current.updated_at = chrono::Utc::now().to_rfc3339();
        }
        session.active = true;
        session.status = "recovering".to_string();
        proj.execution_session = Some(session.clone());
        set_autopilot_recovering(&mut proj, "正在重建验收证据并重新测试");
        touch(&mut proj);
        crate::save_project(&proj)?;
        drop(pipeline_guard);
        return run_recovery_retest(
            &pipeline_state,
            &project_name,
            &session,
            &authorized_paths,
            &recovery.execution_id,
            None,
            RecoveryRetestKind::Full,
        )
        .await;
    }
    if recovery.error_kind == project::RecoveryErrorKind::EvidenceInsufficient {
        let pending = if recovery.pending_evidence_criteria.is_empty() {
            pending_evidence_criteria(&subtask.acceptance_ledger)
        } else {
            recovery.pending_evidence_criteria.clone()
        };
        let strategy = if recovery.evidence_rebuild_attempts >= MAX_EVIDENCE_REBUILD_ATTEMPTS
            || pending.is_empty()
        {
            None
        } else {
            next_evidence_strategy(&recovery)
        };
        let Some(strategy) = strategy else {
            let message = if pending.is_empty() {
                "没有可定向补证的验收项，等待人工补充证据"
            } else {
                "两次定向补证后验收证据仍不足，等待人工处理"
            };
            mark_waiting_human(
                &mut proj,
                project::RecoveryErrorKind::EvidenceInsufficient,
                message,
            );
            write_recovery_history(
                &mut proj,
                "error",
                project::ExecutionEventType::EvidenceStillInsufficient,
                message.to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        };
        if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
            current.phase = project::RecoveryPhase::Retesting;
            current.evidence_rebuild_attempted = true;
            current.evidence_rebuild_attempts = current.evidence_rebuild_attempts.saturating_add(1);
            current.pending_evidence_criteria = pending.clone();
            current.evidence_strategies.push(strategy.clone());
            current.updated_at = chrono::Utc::now().to_rfc3339();
        }
        session.active = true;
        session.status = "recovering".to_string();
        proj.execution_session = Some(session.clone());
        set_autopilot_recovering(&mut proj, "正在补充验收证据");
        write_recovery_history(
            &mut proj,
            "info",
            project::ExecutionEventType::EvidenceRebuildStarted,
            format!("开始 {:?} 补证，验收项：{:?}", strategy, pending),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        touch(&mut proj);
        crate::save_project(&proj)?;
        drop(pipeline_guard);
        return run_recovery_retest(
            &pipeline_state,
            &project_name,
            &session,
            &authorized_paths,
            &recovery.execution_id,
            Some(crate::test_runner::ReviewEvidenceRequest {
                strategy,
                target_criterion_indices: pending,
            }),
            RecoveryRetestKind::Full,
        )
        .await;
    }
    if matches!(
        recovery.error_kind,
        project::RecoveryErrorKind::ReviewTransientFailure
            | project::RecoveryErrorKind::ReviewProtocolFailure
    ) {
        if !validation_retry_due(&recovery) {
            return Ok(proj);
        }
        if recovery.validation_retry_count >= recovery.max_validation_retries {
            mark_waiting_human(&mut proj, recovery.error_kind, "AI 审查验证重试次数已用尽");
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
            current.validation_retry_count = current.validation_retry_count.saturating_add(1);
            current.next_validation_retry_at = None;
            current.phase = project::RecoveryPhase::Retesting;
            current
                .validation_strategies
                .push(project::ValidationRetryStrategy::ReviewRequestRetry);
            current.updated_at = chrono::Utc::now().to_rfc3339();
        }
        session.active = true;
        session.status = "recovering".to_string();
        session.verification_stage = project::VerificationStage::ReviewRetry;
        session.state_entered_at = chrono::Utc::now().to_rfc3339();
        proj.execution_session = Some(session.clone());
        set_autopilot_recovering(
            &mut proj,
            &format!(
                "正在重新请求 AI 审查（{}/{}）",
                recovery.validation_retry_count.saturating_add(1),
                recovery.max_validation_retries
            ),
        );
        write_recovery_history(
            &mut proj,
            "info",
            project::ExecutionEventType::ReviewRequested,
            format!(
                "重新请求 AI 审查：第 {}/{} 次；沿用既有代码、测试事实和验收契约",
                recovery.validation_retry_count.saturating_add(1),
                recovery.max_validation_retries
            ),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        touch(&mut proj);
        crate::save_project(&proj)?;
        drop(pipeline_guard);
        return run_recovery_retest(
            &pipeline_state,
            &project_name,
            &session,
            &authorized_paths,
            &recovery.execution_id,
            None,
            RecoveryRetestKind::ReviewOnly,
        )
        .await;
    }
    if recovery.phase == project::RecoveryPhase::Replanning {
        match replan_current_subtask(&mut proj, &recovery, &session, &subtask, &authorized_paths)
            .await
        {
            Ok(()) => {}
            Err(error) => {
                mark_waiting_human(&mut proj, project::RecoveryErrorKind::HumanRequired, &error);
                write_recovery_history(
                    &mut proj,
                    "error",
                    project::ExecutionEventType::RecoveryExhausted,
                    error,
                    Some(&session.milestone_id),
                    Some(&session.mid_stage_id),
                    Some(&session.subtask_id),
                );
            }
        }
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }
    if recovery.attempt >= recovery.max_attempts {
        mark_waiting_human(&mut proj, recovery.error_kind, "自动修复次数已用尽");
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }

    let diagnosis = build_diagnosis(&proj, &recovery, &subtask, &authorized_paths);
    recovery.attempt = recovery.attempt.saturating_add(1);
    recovery.phase = project::RecoveryPhase::Repairing;
    let replan_execution = recovery.replan_attempted;
    if replan_execution {
        recovery.replan_execution_attempted = true;
    }
    recovery.last_diagnosis = diagnosis.clone();
    recovery.updated_at = chrono::Utc::now().to_rfc3339();

    write_recovery_history(
        &mut proj,
        "info",
        project::ExecutionEventType::ErrorDiagnosed,
        format!("错误诊断完成：{:?}", recovery.error_kind),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );

    if matches!(
        recovery.error_kind,
        project::RecoveryErrorKind::ScopeViolation
            | project::RecoveryErrorKind::StateConflict
            | project::RecoveryErrorKind::WorkspaceError
            | project::RecoveryErrorKind::TestUnavailable
            | project::RecoveryErrorKind::HumanRequired
    ) {
        if recovery.error_kind == project::RecoveryErrorKind::ScopeViolation {
            let target = if recovery.baseline_commit.is_empty() {
                "HEAD"
            } else {
                &recovery.baseline_commit
            };
            if let Err(error) = pipeline::restore_git_execution_baseline(&proj.project_path, target)
            {
                mark_waiting_human(
                    &mut proj,
                    project::RecoveryErrorKind::WorkspaceError,
                    &format!("范围越界且基线恢复失败：{}", error),
                );
                crate::save_project(&proj)?;
                return crate::load_project(&project_name);
            }
            reset_subtask_to_pending(&mut proj, &session);
            preserve_recovery_session(&mut proj, &session, &recovery.execution_id);
        }
        proj.workflow_state.recovery_state = Some(recovery.clone());
        mark_waiting_human(
            &mut proj,
            recovery.error_kind,
            "该错误已完成安全收尾，需要人工处理后继续",
        );
        write_recovery_history(
            &mut proj,
            "error",
            project::ExecutionEventType::RecoveryExhausted,
            "自动恢复停止，等待人工处理".to_string(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }

    if recovery.error_kind == project::RecoveryErrorKind::ExecutionError {
        let target = if recovery.baseline_commit.is_empty() {
            "HEAD"
        } else {
            &recovery.baseline_commit
        };
        if let Err(error) = pipeline::restore_git_execution_baseline(&proj.project_path, target) {
            proj.workflow_state.recovery_state = Some(recovery);
            mark_waiting_human(
                &mut proj,
                project::RecoveryErrorKind::WorkspaceError,
                &format!("执行基线恢复失败：{}", error),
            );
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        reset_subtask_to_pending(&mut proj, &session);
    }

    let prepared_engine = match crate::engine::prepare_engine(&session.engine_snapshot).await {
        Ok(prepared) => prepared,
        Err(error) => {
            let message = format!("准备自动修复执行引擎失败：{error}");
            wait_for_engine_snapshot_confirmation(&mut proj, &mut recovery, &session, &message);
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
    };
    if prepared_engine.health.status.blocks_execution() {
        let message = format!("自动修复执行引擎不可用：{}", prepared_engine.health.message);
        wait_for_engine_snapshot_confirmation(&mut proj, &mut recovery, &session, &message);
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }
    if let Some(reason) = execution_snapshot_mismatch(
        &session,
        prepared_engine.settings(),
        &prepared_engine.health,
    ) {
        let message = format!("执行设置快照需要用户确认：{reason}");
        wait_for_engine_snapshot_confirmation(&mut proj, &mut recovery, &session, &message);
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }

    recovery.checkpoint_id =
        crate::recovery_checkpoint::create(&proj.project_path, &authorized_paths)?;

    let recovery_execution_id = format!(
        "recovery-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    recovery.execution_id = recovery_execution_id.clone();
    proj.workflow_state.recovery_state = Some(recovery.clone());
    session.execution_id = recovery_execution_id.clone();
    session.active = true;
    session.status = "recovering".to_string();
    session.failure_message.clear();
    session.state_entered_at = chrono::Utc::now().to_rfc3339();
    proj.execution_session = Some(session.clone());
    set_subtask_running(&mut proj, &session);
    set_autopilot_recovering(
        &mut proj,
        &format!(
            "正在执行第 {}/{} 次修复",
            recovery.attempt, recovery.max_attempts
        ),
    );
    write_recovery_history(
        &mut proj,
        "info",
        if replan_execution {
            project::ExecutionEventType::ReplanExecutionStarted
        } else {
            project::ExecutionEventType::RepairAttemptStarted
        },
        if replan_execution {
            format!(
                "开始执行重规划后的当前小阶段（{}）",
                session.engine_snapshot.provider.display_name(),
            )
        } else {
            format!(
                "开始第 {}/{} 次自动修复（{}）",
                recovery.attempt,
                recovery.max_attempts,
                session.engine_snapshot.provider.display_name(),
            )
        },
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    touch(&mut proj);
    crate::save_project(&proj)?;

    *pipeline_guard = Some(PipelineState {
        execution_id: recovery_execution_id.clone(),
        mid_stage_id: session.mid_stage_id.clone(),
        status: PipelineStatus::Running,
        current_subtask_index: session.subtask_index,
        total_subtasks: session.total_subtasks,
        subtask_statuses: vec![SubtaskStatusItem {
            subtask_id: session.subtask_id.clone(),
            title: session.subtask_title.clone(),
            status: "repairing".to_string(),
            test_result: None,
            retry_count: recovery.attempt,
        }],
        current_log: format!(
            "正在执行第 {}/{} 次修复",
            recovery.attempt, recovery.max_attempts
        ),
        last_error: None,
        child_pid: None,
        project_name: project_name.clone(),
        milestone_id: session.milestone_id.clone(),
        plan_revision: session.plan_revision,
        current_subtask_id: session.subtask_id.clone(),
        awaiting_confirmation: false,
        log_history: vec![],
    });
    drop(pipeline_guard);

    let prompt = repair_prompt(&recovery, &subtask, &diagnosis);
    let repair_result = crate::engine::execute(
        prepared_engine,
        crate::engine::ExecutionRequest {
            project_path: proj.project_path.clone(),
            prompt,
            authorized_paths: authorized_paths.clone(),
            subtask_id: session.subtask_id.clone(),
            execution_id: recovery_execution_id.clone(),
        },
        pipeline_state.clone(),
    )
    .await;

    let mut pipeline_guard = pipeline_state.lock().await;
    let mut proj = crate::load_project(&project_name)?;
    let current_matches = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|current| current.execution_id == recovery_execution_id);
    if !current_matches {
        if !recovery.checkpoint_id.is_empty() {
            let _ = crate::recovery_checkpoint::discard(&recovery.checkpoint_id);
        }
        return crate::load_project(&project_name);
    }

    let repair_result = match repair_result {
        Ok(result) if result.success => result,
        Ok(result) => {
            let engine_failure_kind = result
                .engine_failure_kind
                .clone()
                .unwrap_or(project::EngineFailureKind::TaskExecutionError);
            let message = if result.error_log.is_empty() {
                format!(
                    "{} 修复进程非零退出",
                    session.engine_snapshot.provider.display_name()
                )
            } else {
                result.error_log.clone()
            };
            if crate::engine::blocks_code_recovery(&engine_failure_kind) {
                handle_repair_engine_block(
                    &mut proj,
                    &session,
                    &recovery_execution_id,
                    &message,
                    engine_failure_kind,
                    Some(result),
                    &mut pipeline_guard,
                )?;
            } else {
                handle_repair_execution_failure(
                    &mut proj,
                    &session,
                    &recovery_execution_id,
                    &message,
                    &mut pipeline_guard,
                )?;
            }
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        Err(crate::engine::EngineError::Cancelled) => {
            finish_repair_checkpoint(&mut proj, true)?;
            mark_waiting_human(
                &mut proj,
                project::RecoveryErrorKind::HumanRequired,
                "自动修复被用户暂停",
            );
            set_pipeline_terminal(
                &mut pipeline_guard,
                &recovery_execution_id,
                None,
                Some("自动修复被用户暂停"),
            );
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        Err(crate::engine::EngineError::Timeout) => {
            handle_repair_engine_block(
                &mut proj,
                &session,
                &recovery_execution_id,
                "自动修复执行超时",
                project::EngineFailureKind::Timeout,
                None,
                &mut pipeline_guard,
            )?;
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        Err(error) => {
            let message = error.to_string();
            let kind = crate::engine::classify_process_failure(None, &message, "");
            if crate::engine::blocks_code_recovery(&kind) {
                handle_repair_engine_block(
                    &mut proj,
                    &session,
                    &recovery_execution_id,
                    &message,
                    kind,
                    None,
                    &mut pipeline_guard,
                )?;
            } else {
                handle_repair_execution_failure(
                    &mut proj,
                    &session,
                    &recovery_execution_id,
                    &message,
                    &mut pipeline_guard,
                )?;
            }
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
    };

    let out_of_scope =
        crate::plan_contract::out_of_scope_changes(&repair_result.file_changes, &authorized_paths);
    if !out_of_scope.is_empty() {
        let target = if recovery.baseline_commit.is_empty() {
            "HEAD"
        } else {
            &recovery.baseline_commit
        };
        let restore = pipeline::restore_git_execution_baseline(&proj.project_path, target);
        finish_repair_checkpoint(&mut proj, restore.is_err())?;
        reset_subtask_to_pending(&mut proj, &session);
        preserve_recovery_session(&mut proj, &session, &recovery_execution_id);
        let message = match restore {
            Ok(()) => format!(
                "自动修复修改了范围外文件并已恢复基线：{}",
                out_of_scope.join("、")
            ),
            Err(error) => format!(
                "自动修复修改了范围外文件且基线恢复失败：{}；{}",
                out_of_scope.join("、"),
                error
            ),
        };
        mark_waiting_human(
            &mut proj,
            project::RecoveryErrorKind::ScopeViolation,
            &message,
        );
        write_recovery_history(
            &mut proj,
            "error",
            project::ExecutionEventType::RecoveryExhausted,
            message.clone(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        set_pipeline_terminal(
            &mut pipeline_guard,
            &recovery_execution_id,
            None,
            Some(&message),
        );
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }

    let previous_execution = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .and_then(|state| state.pending_execution_result.clone())
        .or_else(|| {
            proj.milestones
                .iter()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter()
                        .find(|mid_stage| mid_stage.id == session.mid_stage_id)
                })
                .and_then(|mid_stage| {
                    mid_stage
                        .subtasks
                        .iter()
                        .find(|item| item.id == session.subtask_id)
                })
                .and_then(|item| item.execution_result.clone())
        });
    let merged_execution = merge_execution_result(previous_execution, repair_result);
    if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
        current.phase = project::RecoveryPhase::Retesting;
        current.last_repair_summary = format!(
            "第 {} 次修复完成，修改 {} 个文件",
            current.attempt,
            merged_execution.file_changes.len()
        );
        current.updated_at = chrono::Utc::now().to_rfc3339();
    }
    if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
        current.pending_execution_result = Some(merged_execution);
    }
    write_recovery_history(
        &mut proj,
        "success",
        project::ExecutionEventType::RepairAttemptCompleted,
        format!("第 {} 次自动修复执行完成", recovery.attempt),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    set_autopilot_recovering(&mut proj, "正在重新测试");
    touch(&mut proj);
    crate::save_project(&proj)?;
    drop(pipeline_guard);
    run_recovery_retest(
        &pipeline_state,
        &project_name,
        &session,
        &authorized_paths,
        &recovery_execution_id,
        None,
        RecoveryRetestKind::Full,
    )
    .await
}

fn handle_repair_engine_block(
    proj: &mut project::Project,
    session: &project::ExecutionSession,
    execution_id: &str,
    message: &str,
    engine_failure_kind: project::EngineFailureKind,
    execution_result: Option<project::ExecutionResult>,
    pipeline_state: &mut Option<PipelineState>,
) -> Result<(), String> {
    let baseline = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|state| state.baseline_commit.clone())
        .unwrap_or_default();
    let restore_result = pipeline::restore_git_execution_baseline(
        &proj.project_path,
        if baseline.is_empty() {
            "HEAD"
        } else {
            &baseline
        },
    );
    finish_repair_checkpoint(proj, restore_result.is_err())?;
    reset_subtask_to_pending(proj, session);
    if let Some(result) = execution_result {
        if let Some(item) = proj
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == session.milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter_mut()
                    .find(|mid_stage| mid_stage.id == session.mid_stage_id)
            })
            .and_then(|mid_stage| {
                mid_stage
                    .subtasks
                    .iter_mut()
                    .find(|item| item.id == session.subtask_id)
            })
        {
            item.execution_result = Some(result);
        }
    }
    preserve_recovery_session(proj, session, execution_id);
    let detail = match restore_result {
        Ok(()) => format!("执行引擎阻断，已恢复任务基线：{}", message),
        Err(error) => format!("执行引擎阻断且任务基线恢复失败：{}；{}", message, error),
    };
    if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
        recovery.attempt = recovery.attempt.saturating_sub(1);
        recovery.error_kind = project::RecoveryErrorKind::EngineBlocked;
        recovery.engine_failure_kind = Some(engine_failure_kind);
        recovery.phase = project::RecoveryPhase::WaitingEngine;
        recovery.last_repair_summary = truncate_chars(&detail, 4_000);
        recovery.updated_at = chrono::Utc::now().to_rfc3339();
    }
    if let Some(current_session) = proj.execution_session.as_mut() {
        current_session.failure_message = truncate_chars(&detail, 2_048);
    }
    set_autopilot_waiting(proj, &detail);
    write_recovery_history(
        proj,
        "error",
        project::ExecutionEventType::ExecutionFailed,
        detail.clone(),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    set_pipeline_terminal(pipeline_state, execution_id, None, Some(&detail));
    touch(proj);
    Ok(())
}

fn handle_repair_execution_failure(
    proj: &mut project::Project,
    session: &project::ExecutionSession,
    execution_id: &str,
    message: &str,
    pipeline_state: &mut Option<PipelineState>,
) -> Result<(), String> {
    let baseline = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|state| state.baseline_commit.clone())
        .unwrap_or_default();
    let target = if baseline.is_empty() {
        "HEAD"
    } else {
        &baseline
    };
    let restore_result = pipeline::restore_git_execution_baseline(&proj.project_path, target);
    finish_repair_checkpoint(proj, restore_result.is_err())?;
    reset_subtask_to_pending(proj, session);
    preserve_recovery_session(proj, session, execution_id);

    let (attempt, max_attempts) = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|state| (state.attempt, state.max_attempts))
        .unwrap_or((DEFAULT_MAX_ATTEMPTS, DEFAULT_MAX_ATTEMPTS));
    let detail = match restore_result {
        Ok(()) => format!("自动修复执行失败，已恢复基线：{}", message),
        Err(ref error) => format!("自动修复执行失败且基线恢复失败：{}；{}", message, error),
    };
    let replanned = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|state| state.replan_attempted);
    if restore_result.is_err() || attempt >= max_attempts || replanned {
        if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
            recovery.error_kind = project::RecoveryErrorKind::ExecutionError;
        }
        mark_waiting_human(proj, project::RecoveryErrorKind::ExecutionError, &detail);
        write_recovery_history(
            proj,
            "error",
            project::ExecutionEventType::RecoveryExhausted,
            detail.clone(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
    } else {
        if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
            recovery.error_kind = project::RecoveryErrorKind::ExecutionError;
            recovery.phase = project::RecoveryPhase::Diagnosing;
            recovery.error_signature =
                normalized_signature(&project::RecoveryErrorKind::ExecutionError, message);
            recovery.last_repair_summary = detail.clone();
            recovery.updated_at = chrono::Utc::now().to_rfc3339();
        }
        set_autopilot_recovering(proj, "修复执行失败，准备从基线重新执行");
        touch(proj);
    }
    set_pipeline_terminal(pipeline_state, execution_id, None, Some(&detail));
    Ok(())
}

pub(crate) fn finish_retest(
    proj: &mut project::Project,
    session: &project::ExecutionSession,
    execution_id: &str,
    mut test: project::TestResult,
) -> Result<bool, String> {
    let recovery_is_current = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|current| current.execution_id == execution_id)
        && proj.execution_session.as_ref().is_some_and(|current| {
            current.active
                && current.status.eq_ignore_ascii_case("recovering")
                && current.execution_id == execution_id
        });
    if !recovery_is_current {
        return Err("复测结果属于已失效的恢复会话，已忽略。".to_string());
    }

    let subtask = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter()
                .find(|item| item.id == session.subtask_id)
        })
        .ok_or_else(|| "复测完成后无法定位小阶段。".to_string())?
        .clone();
    let authorized_paths = crate::plan_contract::validate_subtask(&subtask, "恢复复测任务")?;

    test.acceptance_results =
        crate::acceptance::build_ledger(&subtask.acceptance_criteria, &test, &authorized_paths);
    let quality = crate::quality_gate::evaluate(
        Some(&test),
        &test.acceptance_results,
        subtask.acceptance_criteria.len(),
        false,
    );
    let quality_passed = quality.passed();
    test.passed = quality_passed;
    let item = proj
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == session.milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter_mut()
                .find(|mid_stage| mid_stage.id == session.mid_stage_id)
        })
        .and_then(|mid_stage| {
            mid_stage
                .subtasks
                .iter_mut()
                .find(|item| item.id == session.subtask_id)
        })
        .ok_or_else(|| "复测完成后无法定位小阶段。".to_string())?;
    item.status = project::SubtaskStatus::AwaitingConfirmation;
    item.test_result = Some(test.clone());
    item.acceptance_ledger = test.acceptance_results.clone();

    let summary = test_failure_summary(Some(&test), "复测未通过");
    let evidence_recovery = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| {
            recovery.error_kind == project::RecoveryErrorKind::EvidenceInsufficient
                || recovery.evidence_rebuild_attempts > 0
        });
    let validation_recovery = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|recovery| recovery.validation_retry_count > 0);
    write_recovery_history(
        proj,
        if quality_passed { "success" } else { "error" },
        project::ExecutionEventType::RetestCompleted,
        if quality_passed {
            if evidence_recovery {
                "验收证据补充后质量门禁通过".to_string()
            } else if validation_recovery {
                "AI 审查验证重试后质量门禁通过".to_string()
            } else {
                "自动修复复测通过".to_string()
            }
        } else if quality.outcome == crate::quality_gate::QualityGateOutcome::EvidenceInsufficient {
            format!("验收证据仍不足：{}", truncate_chars(&summary, 1_000))
        } else if matches!(
            quality.outcome,
            crate::quality_gate::QualityGateOutcome::ReviewTransientFailure
                | crate::quality_gate::QualityGateOutcome::ReviewProtocolFailure
                | crate::quality_gate::QualityGateOutcome::ReviewServiceBlocked
        ) {
            format!("AI 审查验证未通过：{}", truncate_chars(&summary, 1_000))
        } else {
            format!("自动修复复测未通过：{}", truncate_chars(&summary, 1_000))
        },
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );

    if quality_passed {
        if let Some(checkpoint_id) = proj
            .workflow_state
            .recovery_state
            .as_ref()
            .map(|state| state.checkpoint_id.clone())
            .filter(|id| !id.is_empty())
        {
            crate::recovery_checkpoint::discard(&checkpoint_id)?;
        }
        if let Some(current_session) = proj.execution_session.as_mut() {
            current_session.execution_id = execution_id.to_string();
            current_session.active = true;
            current_session.status = "awaiting_confirmation".to_string();
            current_session.failure_message.clear();
            current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
        }
        let pending_execution = proj
            .workflow_state
            .recovery_state
            .as_mut()
            .and_then(|recovery| recovery.pending_execution_result.take());
        if let Some(item) = proj
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == session.milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter_mut()
                    .find(|mid| mid.id == session.mid_stage_id)
            })
            .and_then(|mid| {
                mid.subtasks
                    .iter_mut()
                    .find(|item| item.id == session.subtask_id)
            })
        {
            if pending_execution.is_some() {
                item.execution_result = pending_execution;
            }
        }
        write_recovery_history(
            proj,
            "success",
            project::ExecutionEventType::RecoverySucceeded,
            if evidence_recovery {
                "验收证据补齐，恢复正常自动驾驶".to_string()
            } else if validation_recovery {
                "AI 审查恢复成功，继续正常自动驾驶".to_string()
            } else {
                "自动修复成功，恢复正常自动驾驶".to_string()
            },
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        if validation_recovery {
            write_recovery_history(
                proj,
                "success",
                project::ExecutionEventType::ValidationRecoverySucceeded,
                "AI 审查验证恢复成功；代码修复次数和 Git 基线保持不变".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
        }
        if let Some(completed_recovery) = proj.workflow_state.recovery_state.clone() {
            let strategy = if completed_recovery.validation_retry_count > 0
                && completed_recovery.attempt == 0
            {
                "沿用代码、测试事实和验收契约，仅重新请求 AI 审查"
            } else if completed_recovery.evidence_rebuild_attempts > 0
                && completed_recovery.attempt == 0
            {
                "按未知验收项执行两级定向补证"
            } else if completed_recovery.replan_attempted {
                "受限计划补丁后从基线完整重执行"
            } else {
                "按验收差异执行受限代码修复"
            };
            crate::recovery_learning::record(
                proj,
                &completed_recovery,
                &subtask,
                strategy,
                true,
                &format!(
                    "保持文件范围 [{}] 与精确标识符 [{}]",
                    subtask.allowed_file_paths.join("、"),
                    subtask.required_identifiers.join("、")
                ),
            );
        }
        proj.workflow_state.recovery_state = None;
        if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
            autopilot.run_status = project::AutopilotRunStatus::Running;
            autopilot.last_action = if evidence_recovery {
                "验收证据补齐，继续执行".to_string()
            } else if validation_recovery {
                "AI 审查恢复成功，继续执行".to_string()
            } else {
                "自动修复成功，继续执行".to_string()
            };
            autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
            autopilot.error_message.clear();
            autopilot.recovery_action = project::AutopilotRecoveryAction::None;
        }
        touch(proj);
        return Ok(false);
    }

    if matches!(
        quality.outcome,
        crate::quality_gate::QualityGateOutcome::ReviewTransientFailure
            | crate::quality_gate::QualityGateOutcome::ReviewProtocolFailure
            | crate::quality_gate::QualityGateOutcome::ReviewServiceBlocked
            | crate::quality_gate::QualityGateOutcome::AutomatedTestUnavailable
            | crate::quality_gate::QualityGateOutcome::TestUnavailable
    ) {
        let next_kind = quality.recovery_error_kind(Some(&test));
        let retry_limit = validation_retry_limit(&next_kind);
        let mut retry_scheduled = false;
        if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
            recovery.error_kind = next_kind.clone();
            recovery.error_signature = normalized_signature(&next_kind, &summary);
            recovery.original_test_failure = truncate_chars(&summary, 4_000);
            recovery.active_issues.clear();
            recovery.rollback_retest_pending = false;
            append_failure_history(recovery, &summary);
            if next_kind == project::RecoveryErrorKind::ReviewProtocolFailure {
                record_review_protocol_strategies(recovery, &test);
            }
            if let Some(limit) = retry_limit {
                recovery.max_validation_retries = limit;
                if recovery.validation_retry_count < limit {
                    recovery.phase = project::RecoveryPhase::Retesting;
                    schedule_next_validation_retry(recovery);
                    retry_scheduled = true;
                } else {
                    recovery.phase = project::RecoveryPhase::WaitingHuman;
                    recovery.next_validation_retry_at = None;
                }
            } else {
                recovery.phase = project::RecoveryPhase::WaitingHuman;
                recovery.next_validation_retry_at = None;
            }
            recovery.updated_at = chrono::Utc::now().to_rfc3339();
        }
        if let Some(current_session) = proj.execution_session.as_mut() {
            current_session.execution_id = execution_id.to_string();
            current_session.active = true;
            current_session.status = if retry_scheduled {
                "recovering".to_string()
            } else {
                "quality_blocked".to_string()
            };
            current_session.verification_stage = test.verification_stage.clone();
            current_session.failure_message = truncate_chars(&summary, 2_048);
            current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
        }
        if retry_scheduled {
            let (count, limit, retry_at) = proj
                .workflow_state
                .recovery_state
                .as_ref()
                .map(|recovery| {
                    (
                        recovery.validation_retry_count,
                        recovery.max_validation_retries,
                        recovery.next_validation_retry_at.clone(),
                    )
                })
                .unwrap_or_default();
            set_autopilot_recovering(
                proj,
                &format!("AI 审查仍不可用，等待第 {}/{} 次验证重试", count + 1, limit),
            );
            if let Some(retry_at) = retry_at {
                write_recovery_history(
                    proj,
                    "info",
                    project::ExecutionEventType::ValidationRetryScheduled,
                    format!(
                        "验证重试已重新安排：第 {}/{} 次，最早执行时间 {}",
                        count.saturating_add(1),
                        limit,
                        retry_at
                    ),
                    Some(&session.milestone_id),
                    Some(&session.mid_stage_id),
                    Some(&session.subtask_id),
                );
            }
        } else {
            let message = match next_kind {
                project::RecoveryErrorKind::ReviewServiceBlocked => {
                    "AI 审查认证或额度异常，等待人工处理"
                }
                project::RecoveryErrorKind::ReviewProtocolFailure => {
                    "AI 审查结果格式持续异常，验证重试已耗尽"
                }
                project::RecoveryErrorKind::ReviewTransientFailure => {
                    "AI 审查服务连续不可用，验证重试已耗尽"
                }
                project::RecoveryErrorKind::AutomatedTestUnavailable => {
                    "自动化测试环境不可用，等待人工处理"
                }
                _ => "验证服务不可用，等待人工处理",
            };
            set_autopilot_waiting(proj, message);
        }
        touch(proj);
        return Ok(false);
    }

    if quality.outcome == crate::quality_gate::QualityGateOutcome::EvidenceInsufficient {
        let pending = pending_evidence_criteria(&test.acceptance_results);
        let evidence_attempts = proj
            .workflow_state
            .recovery_state
            .as_ref()
            .map(|recovery| recovery.evidence_rebuild_attempts)
            .unwrap_or_default();
        let evidence_exhausted =
            pending.is_empty() || evidence_attempts >= MAX_EVIDENCE_REBUILD_ATTEMPTS;
        if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
            recovery.error_kind = project::RecoveryErrorKind::EvidenceInsufficient;
            recovery.phase = if evidence_exhausted {
                project::RecoveryPhase::WaitingHuman
            } else {
                project::RecoveryPhase::Retesting
            };
            recovery.pending_evidence_criteria = pending;
            recovery.active_issues.clear();
            recovery.rollback_retest_pending = false;
            recovery.original_test_failure = truncate_chars(&summary, 4_000);
            append_failure_history(recovery, &summary);
            recovery.updated_at = chrono::Utc::now().to_rfc3339();
        }
        if let Some(current_session) = proj.execution_session.as_mut() {
            current_session.execution_id = execution_id.to_string();
            current_session.active = true;
            current_session.status = if evidence_exhausted {
                "quality_blocked".to_string()
            } else {
                "recovering".to_string()
            };
            current_session.failure_message = truncate_chars(&summary, 2_048);
            current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
        }
        if evidence_exhausted {
            set_autopilot_waiting(proj, "验收证据仍不足，等待人工处理");
        } else {
            set_autopilot_recovering(proj, "验收证据仍不足，准备定向补证");
        }
        touch(proj);
        return Ok(false);
    }

    let mut next_kind = quality.recovery_error_kind(Some(&test));
    let next_issues = recovery_issues(&test, &subtask, &authorized_paths);
    let changed_files = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .and_then(|recovery| recovery.pending_execution_result.as_ref())
        .or(subtask.execution_result.as_ref())
        .map(|result| result.file_changes.clone())
        .unwrap_or_default();
    let mut next_phase = project::RecoveryPhase::Diagnosing;
    let mut contradictory_criteria = Vec::new();
    if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
        recovery.rollback_retest_pending = false;
        let previous_issues = recovery.active_issues.clone();
        let previous_ids: BTreeSet<String> = recovery
            .active_issues
            .iter()
            .map(|issue| issue.id.clone())
            .collect();
        let next_ids: BTreeSet<String> = next_issues.iter().map(|issue| issue.id.clone()).collect();
        let resolved_issue_ids = previous_ids
            .difference(&next_ids)
            .cloned()
            .collect::<Vec<_>>();
        let remaining_issue_ids = previous_ids
            .intersection(&next_ids)
            .cloned()
            .collect::<Vec<_>>();
        let regressed_issue_ids = next_ids
            .difference(&previous_ids)
            .cloned()
            .collect::<Vec<_>>();
        let previously_resolved = recovery
            .attempt_history
            .iter()
            .flat_map(|record| record.resolved_issue_ids.iter())
            .collect::<BTreeSet<_>>();
        let oscillating_ids = regressed_issue_ids
            .iter()
            .filter(|id| previously_resolved.contains(id))
            .cloned()
            .collect::<BTreeSet<_>>();
        if !oscillating_ids.is_empty() {
            next_kind = project::RecoveryErrorKind::ValidationOscillation;
            contradictory_criteria.extend(
                next_issues
                    .iter()
                    .filter(|issue| oscillating_ids.contains(&issue.id))
                    .filter_map(|issue| issue.criterion_index),
            );
        }
        let made_progress = !previous_ids.is_empty()
            && !resolved_issue_ids.is_empty()
            && next_ids.len() < previous_ids.len();
        let introduced_regression = !regressed_issue_ids.is_empty();
        let regression_count = regressed_issue_ids.len();
        let attempt_summary = format!(
            "第 {} 次复测：解决 {} 项，剩余 {} 项，新增 {} 项",
            recovery.attempt,
            resolved_issue_ids.len(),
            next_ids.len(),
            regressed_issue_ids.len(),
        );
        recovery
            .attempt_history
            .push(project::RecoveryAttemptRecord {
                attempt: recovery.attempt,
                issue_ids: previous_ids.into_iter().collect(),
                resolved_issue_ids,
                remaining_issue_ids,
                regressed_issue_ids,
                changed_files,
                made_progress,
                summary: attempt_summary.clone(),
                recorded_at: chrono::Utc::now().to_rfc3339(),
            });
        if recovery.attempt_history.len() > MAX_FAILURE_HISTORY {
            recovery
                .attempt_history
                .drain(0..recovery.attempt_history.len() - MAX_FAILURE_HISTORY);
        }
        let previous_failure = recovery.original_test_failure.clone();
        recovery.original_test_failure = truncate_chars(&summary, 4_000);
        append_failure_history(recovery, &summary);
        recovery.active_issues = next_issues;
        recovery.last_repair_summary = attempt_summary;
        recovery.updated_at = chrono::Utc::now().to_rfc3339();
        let checkpoint_id = recovery.checkpoint_id.clone();
        recovery.checkpoint_id.clear();
        if introduced_regression && !checkpoint_id.is_empty() {
            crate::recovery_checkpoint::restore(&checkpoint_id)?;
            if let Some(item) = proj
                .milestones
                .iter_mut()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter_mut()
                        .find(|mid| mid.id == session.mid_stage_id)
                })
                .and_then(|mid| {
                    mid.subtasks
                        .iter_mut()
                        .find(|item| item.id == session.subtask_id)
                })
            {
                item.test_report.clear();
                item.test_result = None;
                item.acceptance_ledger.clear();
                item.human_verification = None;
                item.status = project::SubtaskStatus::Executing;
            }
            recovery.active_issues = previous_issues;
            recovery.original_test_failure = previous_failure;
            recovery.rollback_retest_pending = true;
            recovery.pending_execution_result = None;
            recovery.phase = project::RecoveryPhase::Retesting;
            if let Some(current_session) = proj.execution_session.as_mut() {
                current_session.status = "recovering".to_string();
                current_session.active = true;
            }
            recovery.last_repair_summary = format!(
                "{}；检测到 {} 个新增回归，已撤销本轮修复",
                recovery.last_repair_summary, regression_count
            );
            touch(proj);
            return Ok(true);
        } else if !checkpoint_id.is_empty() {
            crate::recovery_checkpoint::discard(&checkpoint_id)?;
        }
        let accepted_execution = recovery.pending_execution_result.take();
        if let Some(item) = proj
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == session.milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter_mut()
                    .find(|mid| mid.id == session.mid_stage_id)
            })
            .and_then(|mid| {
                mid.subtasks
                    .iter_mut()
                    .find(|item| item.id == session.subtask_id)
            })
        {
            if accepted_execution.is_some() {
                item.execution_result = accepted_execution;
            }
        }
        let next_signature = normalized_signature(&next_kind, &summary);
        let regular_repair_exhausted =
            record_failed_signature(recovery, next_kind.clone(), next_signature);
        next_phase = if next_kind == project::RecoveryErrorKind::PlanFailure {
            project::RecoveryPhase::Replanning
        } else if matches!(
            next_kind,
            project::RecoveryErrorKind::TestUnavailable
                | project::RecoveryErrorKind::EvidenceInsufficient
                | project::RecoveryErrorKind::ValidationFailure
                | project::RecoveryErrorKind::ContractContradiction
                | project::RecoveryErrorKind::ValidationOscillation
        ) || recovery.replan_execution_attempted
        {
            project::RecoveryPhase::WaitingHuman
        } else if regular_repair_exhausted {
            project::RecoveryPhase::Replanning
        } else {
            project::RecoveryPhase::Diagnosing
        };
        recovery.phase = next_phase.clone();
    }

    if !contradictory_criteria.is_empty() {
        if let Some(item) = proj
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == session.milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter_mut()
                    .find(|mid_stage| mid_stage.id == session.mid_stage_id)
            })
            .and_then(|mid_stage| {
                mid_stage
                    .subtasks
                    .iter_mut()
                    .find(|item| item.id == session.subtask_id)
            })
        {
            for ledger in &mut item.acceptance_ledger {
                if contradictory_criteria.contains(&ledger.criterion_index) {
                    ledger.status = project::AcceptanceStatus::Contradictory;
                    ledger.evidence =
                        "该验收项在连续审查中先被解决后再次出现，审查结论发生震荡".to_string();
                    ledger.updated_at = chrono::Utc::now().to_rfc3339();
                }
            }
        }
    }

    if let Some(current_session) = proj.execution_session.as_mut() {
        current_session.execution_id = execution_id.to_string();
        current_session.active = true;
        current_session.status = match next_phase {
            project::RecoveryPhase::WaitingHuman => "quality_blocked".to_string(),
            project::RecoveryPhase::Replanning => "replanning".to_string(),
            _ => "awaiting_confirmation".to_string(),
        };
        current_session.failure_message = truncate_chars(&summary, 2_048);
        current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
    }

    if next_phase == project::RecoveryPhase::WaitingHuman {
        let waiting_message = match next_kind {
            project::RecoveryErrorKind::ContractContradiction => {
                "验收结论与有效阻断证据冲突，等待人工判断"
            }
            project::RecoveryErrorKind::ValidationOscillation => "验收结论反复变化，已停止机械重试",
            project::RecoveryErrorKind::TestUnavailable => "测试或代码审查服务不可用，等待人工处理",
            _ => "代码自动恢复达到停止条件，等待人工处理",
        };
        if let Some(failed_recovery) = proj.workflow_state.recovery_state.clone() {
            crate::recovery_learning::record(
                proj,
                &failed_recovery,
                &subtask,
                if failed_recovery.replan_attempted {
                    "受限计划补丁后从基线完整重执行"
                } else {
                    "按验收差异执行受限代码修复"
                },
                false,
                if failed_recovery.error_kind == project::RecoveryErrorKind::ValidationFailure {
                    "先重建或校正验收证据，禁止继续修改代码"
                } else {
                    "该策略未取得稳定进展，后续不得机械重复"
                },
            );
        }
        set_autopilot_waiting(proj, waiting_message);
        write_recovery_history(
            proj,
            "error",
            project::ExecutionEventType::RecoveryExhausted,
            waiting_message.to_string(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
    } else if next_phase == project::RecoveryPhase::Replanning {
        set_autopilot_recovering(proj, "常规修复耗尽，正在重新规划当前任务");
        write_recovery_history(
            proj,
            "info",
            project::ExecutionEventType::ReplanStarted,
            "常规修复耗尽，开始当前小阶段受限重规划".to_string(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
    } else {
        set_autopilot_recovering(proj, "复测未通过，准备下一次受限修复");
    }
    touch(proj);
    Ok(false)
}

fn changed_paths(project_path: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-z"])
        .current_dir(project_path)
        .output()
        .map_err(|error| format!("读取工作区变更失败：{}", error))?;
    if !output.status.success() {
        return Err(format!(
            "读取工作区变更失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let entries = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        if entry.len() < 4 {
            index += 1;
            continue;
        }
        let status = &entry[..2];
        paths.push(String::from_utf8_lossy(&entry[3..]).to_string());
        if (status.contains(&b'R') || status.contains(&b'C')) && index + 1 < entries.len() {
            index += 1;
            paths.push(String::from_utf8_lossy(entries[index]).to_string());
        }
        index += 1;
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn validate_human_acceptance(
    subtask: &project::Subtask,
    resolution: &str,
    reason: &str,
    accepted_criteria: &[u32],
) -> Result<project::HumanResolution, String> {
    if reason.trim().is_empty() {
        return Err("人工决策必须填写依据。".to_string());
    }
    if subtask
        .execution_result
        .as_ref()
        .is_none_or(|result| !result.success)
    {
        return Err("执行引擎没有成功完成任务，不能通过人工核验或接受代码偏差。".to_string());
    }
    let human_resolution = if resolution == "accept_deviation" {
        project::HumanResolution::AcceptDeviation
    } else {
        project::HumanResolution::ConfirmActualPass
    };
    if human_resolution == project::HumanResolution::AcceptDeviation {
        if accepted_criteria.is_empty() {
            return Err("接受偏差必须选择至少一个验收项。".to_string());
        }
        if accepted_criteria
            .iter()
            .any(|index| *index == 0 || *index as usize > subtask.acceptance_criteria.len())
        {
            return Err("接受偏差包含无效验收项编号。".to_string());
        }
    }
    Ok(human_resolution)
}

fn validate_skip_dependencies(
    skipped: &project::Subtask,
    remaining: &[project::Subtask],
) -> Result<String, String> {
    let hard_dependents = remaining
        .iter()
        .filter(|item| item.depends_on.contains(&skipped.id))
        .map(|item| item.title.clone())
        .collect::<Vec<_>>();
    if !hard_dependents.is_empty() {
        return Err(format!(
            "后续任务存在硬依赖，不能跳过：{}",
            hard_dependents.join("、")
        ));
    }
    if !remaining.is_empty()
        && remaining
            .iter()
            .any(|item| item.depends_on.is_empty() && item.dependency_notes.trim().is_empty())
    {
        return Err("旧计划没有显式依赖契约，无法证明跳过安全；请先重新校准后续任务。".to_string());
    }
    Ok(if remaining.is_empty() {
        "没有后续任务".to_string()
    } else {
        "后续任务显式声明不依赖当前任务".to_string()
    })
}

#[tauri::command]
pub(crate) async fn resolve_human_recovery(
    state: tauri::State<'_, AppState>,
    project_name: String,
    resolution: String,
    reason: String,
    accepted_criteria: Option<Vec<u32>>,
) -> Result<project::Project, String> {
    let _pipeline_guard = state.pipeline_state.lock().await;
    let mut proj = crate::load_project(&project_name)?;
    let (recovery, mut session, subtask) = current_recovery_context(&proj)?;
    if recovery.phase == project::RecoveryPhase::WaitingEngine
        || recovery.error_kind == project::RecoveryErrorKind::EngineBlocked
    {
        return Err("执行引擎仍处于阻断状态；请通过引擎恢复入口检查或切换引擎。".to_string());
    }
    let authorized_paths = crate::plan_contract::validate_subtask(&subtask, "人工恢复任务")?;

    match resolution.as_str() {
        "restore_and_retry" => {
            let baseline = proj
                .workflow_state
                .recovery_state
                .as_ref()
                .map(|current| current.baseline_commit.clone())
                .unwrap_or_default();
            let target = if baseline.is_empty() {
                "HEAD"
            } else {
                &baseline
            };
            pipeline::restore_git_execution_baseline(&proj.project_path, target)?;
            reset_subtask_to_pending(&mut proj, &session);
            proj.execution_session = None;
            proj.workflow_state.recovery_state = None;
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::Running;
                autopilot.last_action = "已恢复基线，重新执行当前小阶段".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                autopilot.error_message.clear();
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            }
        }
        "human_override" | "confirm_actual_pass" | "accept_deviation" => {
            let accepted = accepted_criteria.unwrap_or_default();
            let human_resolution =
                validate_human_acceptance(&subtask, &resolution, &reason, &accepted)?;
            let original_failure =
                test_failure_summary(subtask.test_result.as_ref(), "没有可用的自动化测试结果");
            let item = proj
                .milestones
                .iter_mut()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter_mut()
                        .find(|mid_stage| mid_stage.id == session.mid_stage_id)
                })
                .and_then(|mid_stage| {
                    mid_stage
                        .subtasks
                        .iter_mut()
                        .find(|item| item.id == session.subtask_id)
                })
                .ok_or_else(|| "无法定位人工核验的小阶段。".to_string())?;
            item.status = project::SubtaskStatus::AwaitingConfirmation;
            for ledger in &mut item.acceptance_ledger {
                if accepted.contains(&ledger.criterion_index) {
                    ledger.status = project::AcceptanceStatus::AcceptedDeviation;
                    ledger.evidence = reason.trim().to_string();
                    ledger.updated_at = chrono::Utc::now().to_rfc3339();
                }
            }
            item.human_verification = Some(project::HumanVerification {
                verification_kind: project::VerificationKind::HumanOverride,
                verification_reason: reason.clone(),
                verified_at: chrono::Utc::now().to_rfc3339(),
                original_test_failure: original_failure,
                resolution: human_resolution.clone(),
                accepted_criteria: accepted,
                dependency_check: String::new(),
            });
            if let Some(current_session) = proj.execution_session.as_mut() {
                current_session.status = "awaiting_confirmation".to_string();
                current_session.active = true;
                current_session.failure_message.clear();
            }
            pipeline::write_execution_history_with_source(
                &mut proj,
                "success",
                project::ExecutionEventType::HumanVerificationAccepted,
                project::OperationSource::User,
                format!(
                    "{}：{}",
                    if human_resolution == project::HumanResolution::AcceptDeviation {
                        "接受偏差并继续"
                    } else {
                        "确认实际通过"
                    },
                    reason.trim()
                ),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
            if human_resolution == project::HumanResolution::AcceptDeviation {
                crate::recovery_learning::record_human_constraint(
                    &mut proj,
                    &subtask,
                    "人工接受验收偏差",
                    reason.trim(),
                );
            }
            proj.workflow_state.recovery_state = None;
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::Running;
                autopilot.last_action =
                    if human_resolution == project::HumanResolution::AcceptDeviation {
                        "验收偏差已记录，准备将约束传播到后续任务".to_string()
                    } else {
                        "人工通过证据已记录，继续执行".to_string()
                    };
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                autopilot.error_message.clear();
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            }
        }
        "skip_task" => {
            if reason.trim().is_empty() {
                return Err("跳过任务必须填写原因。".to_string());
            }
            let remaining = proj
                .milestones
                .iter()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter()
                        .find(|mid| mid.id == session.mid_stage_id)
                })
                .map(|mid| {
                    mid.subtasks
                        .iter()
                        .filter(|item| {
                            item.order > subtask.order
                                && item.status == project::SubtaskStatus::Pending
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let dependency_check = validate_skip_dependencies(&subtask, &remaining)?;
            let baseline = proj
                .workflow_state
                .recovery_state
                .as_ref()
                .map(|current| current.baseline_commit.clone())
                .unwrap_or_default();
            pipeline::restore_git_execution_baseline(
                &proj.project_path,
                if baseline.is_empty() {
                    "HEAD"
                } else {
                    &baseline
                },
            )?;
            let item = proj
                .milestones
                .iter_mut()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter_mut()
                        .find(|mid| mid.id == session.mid_stage_id)
                })
                .and_then(|mid| {
                    mid.subtasks
                        .iter_mut()
                        .find(|item| item.id == session.subtask_id)
                })
                .ok_or_else(|| "无法定位要跳过的小阶段。".to_string())?;
            item.status = project::SubtaskStatus::Skipped;
            item.execution_result = None;
            item.test_result = None;
            item.human_verification = Some(project::HumanVerification {
                verification_kind: project::VerificationKind::HumanOverride,
                verification_reason: reason.clone(),
                verified_at: chrono::Utc::now().to_rfc3339(),
                original_test_failure: test_failure_summary(
                    subtask.test_result.as_ref(),
                    "任务未完成",
                ),
                resolution: project::HumanResolution::SkipTask,
                accepted_criteria: vec![],
                dependency_check,
            });
            proj.execution_session = None;
            proj.workflow_state.recovery_state = None;
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::Running;
                autopilot.last_action = "当前任务已跳过，后续执行前将重新扫描事实".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                autopilot.error_message.clear();
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            }
            pipeline::write_execution_history_with_source(
                &mut proj,
                "pause",
                project::ExecutionEventType::TaskSkipped,
                project::OperationSource::User,
                format!("跳过任务：{}；{}", subtask.title, reason.trim()),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
            crate::recovery_learning::record_human_constraint(
                &mut proj,
                &subtask,
                "依赖检查后跳过任务",
                reason.trim(),
            );
            let (mid_completed, milestone_completed) = pipeline::reconcile_terminal_stage(
                &mut proj,
                &session.milestone_id,
                &session.mid_stage_id,
            );
            if mid_completed {
                pipeline::write_execution_history_with_source(
                    &mut proj,
                    "success",
                    project::ExecutionEventType::MidStageComplete,
                    project::OperationSource::User,
                    "中阶段所有任务已达到终态".to_string(),
                    Some(&session.milestone_id),
                    Some(&session.mid_stage_id),
                    None,
                );
            }
            if milestone_completed {
                pipeline::write_execution_history_with_source(
                    &mut proj,
                    "success",
                    project::ExecutionEventType::AdvanceMilestoneReview,
                    project::OperationSource::User,
                    "所有中阶段已完成，进入大阶段审阅".to_string(),
                    Some(&session.milestone_id),
                    None,
                    None,
                );
            }
        }
        "regenerate_plan" => {
            let recovery = proj
                .workflow_state
                .recovery_state
                .as_mut()
                .ok_or_else(|| "当前没有可重新规划的恢复任务。".to_string())?;
            if recovery.replan_attempted {
                return Err("当前小阶段已经执行过一次受限重规划。".to_string());
            }
            recovery.phase = project::RecoveryPhase::Replanning;
            recovery.updated_at = chrono::Utc::now().to_rfc3339();
            if let Some(current_session) = proj.execution_session.as_mut() {
                current_session.active = true;
                current_session.status = "replanning".to_string();
                current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
            }
            set_autopilot_recovering(&mut proj, "正在重新规划当前任务");
            pipeline::write_execution_history_with_source(
                &mut proj,
                "info",
                project::ExecutionEventType::ReplanStarted,
                project::OperationSource::User,
                "人工请求当前小阶段受限重规划".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
        }
        "revalidate" => {
            if !matches!(
                recovery.error_kind,
                project::RecoveryErrorKind::ReviewServiceBlocked
                    | project::RecoveryErrorKind::ReviewProtocolFailure
                    | project::RecoveryErrorKind::ReviewTransientFailure
            ) {
                return Err("当前阻断不是可单独重新验证的 AI 审查服务问题。".to_string());
            }
            if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
                current.phase = project::RecoveryPhase::Retesting;
                current.next_validation_retry_at = None;
                current.updated_at = chrono::Utc::now().to_rfc3339();
            }
            session.active = true;
            session.status = "recovering".to_string();
            session.verification_stage = project::VerificationStage::ReviewRetry;
            session.state_entered_at = chrono::Utc::now().to_rfc3339();
            proj.execution_session = Some(session.clone());
            set_autopilot_recovering(&mut proj, "人工请求重新验证 AI 审查");
            pipeline::write_execution_history_with_source(
                &mut proj,
                "info",
                project::ExecutionEventType::ReviewRequested,
                project::OperationSource::User,
                "人工请求重新验证 AI 审查；沿用既有代码、测试事实和验收契约".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
            touch(&mut proj);
            crate::save_project(&proj)?;
            drop(_pipeline_guard);
            return run_recovery_retest(
                &state.pipeline_state,
                &project_name,
                &session,
                &authorized_paths,
                &recovery.execution_id,
                None,
                RecoveryRetestKind::ReviewOnly,
            )
            .await;
        }
        "retest" => {
            let changes = changed_paths(&proj.project_path)?;
            let out_of_scope =
                crate::plan_contract::out_of_scope_changes(&changes, &authorized_paths);
            if !out_of_scope.is_empty() {
                return Err(format!(
                    "人工修复包含范围外文件，不能复测：{}",
                    out_of_scope.join("、")
                ));
            }
            if let Some(current) = proj.workflow_state.recovery_state.as_mut() {
                current.phase = project::RecoveryPhase::Retesting;
                current.updated_at = chrono::Utc::now().to_rfc3339();
            }
            set_autopilot_waiting(&mut proj, "人工修复已提交，正在重新测试");
            touch(&mut proj);
            crate::save_project(&proj)?;
            drop(_pipeline_guard);

            let prompt = if subtask.execution_prompt.is_empty() {
                subtask.prompt.clone()
            } else {
                subtask.execution_prompt.clone()
            };
            let mut test = crate::test_runner::check_subtask_with_context(
                &proj.project_path,
                if subtask.goal.is_empty() {
                    &subtask.title
                } else {
                    &subtask.goal
                },
                &session.subtask_id,
                &session.milestone_id,
                &session.mid_stage_id,
                Some(subtask.acceptance_criteria.clone()),
                Some(authorized_paths.clone()),
                Some(prompt),
                None,
            )
            .await
            .unwrap_or(project::TestResult {
                passed: false,
                issues: vec!["测试服务不可用".to_string()],
                suggestion: "请人工核验".to_string(),
                automated_test_status: project::AutomatedTestStatus::Unavailable,
                ..Default::default()
            });
            test.acceptance_results = crate::acceptance::build_ledger(
                &subtask.acceptance_criteria,
                &test,
                &authorized_paths,
            );
            let quality = crate::quality_gate::evaluate(
                Some(&test),
                &test.acceptance_results,
                subtask.acceptance_criteria.len(),
                false,
            );
            test.passed = quality.passed();
            let mut proj = crate::load_project(&project_name)?;
            if quality.passed() {
                let item = proj
                    .milestones
                    .iter_mut()
                    .find(|milestone| milestone.id == session.milestone_id)
                    .and_then(|milestone| {
                        milestone
                            .mid_stages
                            .iter_mut()
                            .find(|mid_stage| mid_stage.id == session.mid_stage_id)
                    })
                    .and_then(|mid_stage| {
                        mid_stage
                            .subtasks
                            .iter_mut()
                            .find(|item| item.id == session.subtask_id)
                    })
                    .ok_or_else(|| "复测完成后无法定位小阶段。".to_string())?;
                item.status = project::SubtaskStatus::AwaitingConfirmation;
                item.test_result = Some(test);
                item.acceptance_ledger = item
                    .test_result
                    .as_ref()
                    .map(|result| result.acceptance_results.clone())
                    .unwrap_or_default();
                if let Some(current_session) = proj.execution_session.as_mut() {
                    current_session.status = "awaiting_confirmation".to_string();
                    current_session.active = true;
                    current_session.failure_message.clear();
                }
                proj.workflow_state.recovery_state = None;
                if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                    autopilot.run_status = project::AutopilotRunStatus::Running;
                    autopilot.last_action = "人工修复复测通过，继续执行".to_string();
                    autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                    autopilot.error_message.clear();
                    autopilot.recovery_action = project::AutopilotRecoveryAction::None;
                }
                pipeline::write_execution_history_with_source(
                    &mut proj,
                    "success",
                    project::ExecutionEventType::RecoverySucceeded,
                    project::OperationSource::User,
                    "人工修复复测通过".to_string(),
                    Some(&session.milestone_id),
                    Some(&session.mid_stage_id),
                    Some(&session.subtask_id),
                );
            } else {
                if let Some(item) = proj
                    .milestones
                    .iter_mut()
                    .find(|milestone| milestone.id == session.milestone_id)
                    .and_then(|milestone| {
                        milestone
                            .mid_stages
                            .iter_mut()
                            .find(|mid_stage| mid_stage.id == session.mid_stage_id)
                    })
                    .and_then(|mid_stage| {
                        mid_stage
                            .subtasks
                            .iter_mut()
                            .find(|item| item.id == session.subtask_id)
                    })
                {
                    item.status = project::SubtaskStatus::AwaitingConfirmation;
                    item.test_result = Some(test.clone());
                    item.acceptance_ledger = test.acceptance_results.clone();
                }
                mark_waiting_human(
                    &mut proj,
                    classify_test_result_with_context(
                        Some(&test),
                        Some(&subtask),
                        &authorized_paths,
                    ),
                    "人工修复后复测仍未通过",
                );
            }
            touch(&mut proj);
            crate::save_project(&proj)?;
            let updated = crate::load_project(&project_name)?;
            state
                .autopilot_runtime
                .start_if_active(state.pipeline_state.clone(), project_name)
                .await?;
            return Ok(updated);
        }
        _ => return Err(format!("未知的人工恢复动作：{}", resolution)),
    }

    touch(&mut proj);
    crate::save_project(&proj)?;
    let updated = crate::load_project(&project_name)?;
    drop(_pipeline_guard);
    state
        .autopilot_runtime
        .start_if_active(state.pipeline_state.clone(), project_name)
        .await?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_subtask() -> project::Subtask {
        project::Subtask {
            id: "subtask-1".to_string(),
            title: "实现默认引擎".to_string(),
            prompt: "实现默认引擎".to_string(),
            status: project::SubtaskStatus::AwaitingConfirmation,
            test_report: String::new(),
            execution_result: None,
            test_result: None,
            retry_count: 0,
            auto_tag: None,
            order: 1,
            goal: "实现默认引擎".to_string(),
            allowed_file_paths: vec!["index.html".to_string()],
            new_file_paths: vec![],
            evidence_files: vec!["index.html".to_string()],
            context_summary: String::new(),
            acceptance_criteria: vec!["对象包含 isDefault 字段".to_string()],
            stop_rules: vec![],
            execution_prompt: "实现 isDefault 字段".to_string(),
            confirmed_by_user: None,
            confirmed_at: None,
            confirmation_notes: None,
            human_verification: None,
            ..Default::default()
        }
    }

    fn evidence_reference(block_id: &str) -> project::ReviewEvidenceReference {
        project::ReviewEvidenceReference {
            block_id: block_id.to_string(),
            source_kind: project::EvidenceSourceKind::CurrentFileSnippet,
            file: "index.html".to_string(),
            start_line: Some(1),
            end_line: Some(3),
        }
    }

    #[test]
    fn evidence_strategies_are_bounded_and_do_not_spend_repair_attempts() {
        let mut recovery = project::RecoveryState {
            attempt: 2,
            replan_attempted: true,
            ..Default::default()
        };
        assert_eq!(
            next_evidence_strategy(&recovery),
            Some(project::ReviewEvidenceStrategy::Targeted)
        );
        recovery.evidence_rebuild_attempts = 1;
        assert_eq!(
            next_evidence_strategy(&recovery),
            Some(project::ReviewEvidenceStrategy::ExpandedTargeted)
        );
        recovery.evidence_rebuild_attempts = 2;
        assert_eq!(next_evidence_strategy(&recovery), None);
        assert_eq!(recovery.attempt, 2);
        assert!(recovery.replan_attempted);
    }

    #[test]
    fn targeted_review_replaces_only_requested_criteria() {
        let previous = project::TestResult {
            criterion_reviews: vec![
                project::CriterionReviewResult {
                    criterion_index: 1,
                    conclusion: project::CriterionReviewConclusion::Satisfied,
                    confidence: 0.9,
                    evidence_references: vec![evidence_reference("E001")],
                    ..Default::default()
                },
                project::CriterionReviewResult {
                    criterion_index: 2,
                    conclusion: project::CriterionReviewConclusion::EvidenceInsufficient,
                    ..Default::default()
                },
            ],
            review_evidence_summary: "standard".to_string(),
            ..Default::default()
        };
        let targeted = project::TestResult {
            criterion_reviews: vec![project::CriterionReviewResult {
                criterion_index: 2,
                conclusion: project::CriterionReviewConclusion::Satisfied,
                confidence: 0.9,
                evidence_references: vec![evidence_reference("E002")],
                ..Default::default()
            }],
            review_evidence_summary: "targeted".to_string(),
            ..Default::default()
        };

        let merged = merge_targeted_review(Some(&previous), targeted, &[2]);

        assert_eq!(merged.criterion_reviews.len(), 2);
        assert_eq!(
            merged.criterion_reviews[0].conclusion,
            project::CriterionReviewConclusion::Satisfied
        );
        assert_eq!(
            merged.criterion_reviews[1].evidence_references[0].block_id,
            "E002"
        );
        assert!(merged.review_evidence_summary.contains("standard"));
        assert!(merged.review_evidence_summary.contains("targeted"));
    }

    #[test]
    fn classifies_structured_test_failures_without_message_parsing() {
        let failed = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Failed,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&failed)),
            project::RecoveryErrorKind::TestFailure
        );

        let unavailable = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Unavailable,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&unavailable)),
            project::RecoveryErrorKind::AutomatedTestUnavailable
        );

        let failed_with_unavailable_review = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Failed,
            warnings: vec!["AI API 调用失败".to_string()],
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&failed_with_unavailable_review)),
            project::RecoveryErrorKind::TestFailure
        );

        let mut review_failure = project::TestResult {
            review_status: project::ReviewStatus::Failed,
            warnings: vec!["AI API 和解析失败文本不得参与分类".to_string()],
            ..Default::default()
        };
        review_failure.review_failure_kind = Some(project::ReviewFailureKind::FieldTypeMismatch);
        assert_eq!(
            classify_test_result(Some(&review_failure)),
            project::RecoveryErrorKind::ReviewProtocolFailure
        );
        review_failure.review_failure_kind = Some(project::ReviewFailureKind::Network);
        assert_eq!(
            classify_test_result(Some(&review_failure)),
            project::RecoveryErrorKind::ReviewTransientFailure
        );
        review_failure.review_failure_kind = Some(project::ReviewFailureKind::Authentication);
        assert_eq!(
            classify_test_result(Some(&review_failure)),
            project::RecoveryErrorKind::ReviewServiceBlocked
        );

        let partial_review = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_evidence_status: project::ReviewEvidenceStatus::Partial,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&partial_review)),
            project::RecoveryErrorKind::EvidenceInsufficient
        );

        let complete_review = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_evidence_status: project::ReviewEvidenceStatus::Complete,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&complete_review)),
            project::RecoveryErrorKind::EvidenceInsufficient
        );

        let plan_failure = project::TestResult {
            passed: false,
            review_evidence_status: project::ReviewEvidenceStatus::Complete,
            issues: vec!["当前任务要求与实际项目结构不匹配".to_string()],
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&plan_failure)),
            project::RecoveryErrorKind::EvidenceInsufficient
        );
    }

    #[test]
    fn old_recovery_state_fields_have_safe_defaults() {
        let value = serde_json::json!({
            "error_kind": "TestFailure",
            "phase": "Diagnosing",
            "attempt": 0,
            "max_attempts": 2,
            "error_signature": "failure",
            "subtask_id": "st-1",
            "execution_id": "exec-1",
            "started_at": "now",
            "updated_at": "now"
        });
        let restored: project::RecoveryState = serde_json::from_value(value).unwrap();
        assert_eq!(restored.repeated_signature_count, 0);
        assert!(restored.baseline_commit.is_empty());
        assert!(restored.engine_failure_kind.is_none());
        assert!(restored.checkpoint_id.is_empty());
        assert!(!restored.rollback_retest_pending);
        assert!(!restored.evidence_rebuild_attempted);
        assert!(restored.pending_execution_result.is_none());
    }

    #[test]
    fn engine_blocked_state_spends_no_repair_or_replan_attempts() {
        let recovery = project::RecoveryState {
            error_kind: project::RecoveryErrorKind::EngineBlocked,
            phase: project::RecoveryPhase::WaitingEngine,
            engine_failure_kind: Some(project::EngineFailureKind::QuotaExceeded),
            ..Default::default()
        };
        assert_eq!(recovery.attempt, 0);
        assert!(!recovery.replan_attempted);
        assert!(!recovery.replan_execution_attempted);
    }

    #[test]
    fn recovery_state_marks_rollback_for_real_retest() {
        let recovery = project::RecoveryState {
            rollback_retest_pending: true,
            pending_execution_result: Some(project::ExecutionResult {
                success: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let value = serde_json::to_value(&recovery).unwrap();
        let restored: project::RecoveryState = serde_json::from_value(value).unwrap();
        assert!(restored.rollback_retest_pending);
        assert!(restored.pending_execution_result.is_some());
    }

    #[test]
    fn human_pass_requires_successful_execution() {
        let mut subtask = contract_subtask();
        subtask.execution_result = Some(project::ExecutionResult {
            success: false,
            ..Default::default()
        });
        assert!(
            validate_human_acceptance(&subtask, "confirm_actual_pass", "manual evidence", &[],)
                .is_err()
        );
        subtask.execution_result.as_mut().unwrap().success = true;
        assert_eq!(
            validate_human_acceptance(&subtask, "confirm_actual_pass", "manual evidence", &[],)
                .unwrap(),
            project::HumanResolution::ConfirmActualPass
        );
    }

    #[test]
    fn accepting_deviation_requires_valid_criteria() {
        let mut subtask = contract_subtask();
        subtask.execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        assert!(
            validate_human_acceptance(&subtask, "accept_deviation", "known deviation", &[],)
                .is_err()
        );
        assert!(
            validate_human_acceptance(&subtask, "accept_deviation", "known deviation", &[2],)
                .is_err()
        );
        assert_eq!(
            validate_human_acceptance(&subtask, "accept_deviation", "known deviation", &[1],)
                .unwrap(),
            project::HumanResolution::AcceptDeviation
        );
    }

    #[test]
    fn skipping_requires_explicit_non_dependency() {
        let skipped = contract_subtask();
        let legacy = project::Subtask {
            id: "next".to_string(),
            title: "legacy next".to_string(),
            ..Default::default()
        };
        assert!(validate_skip_dependencies(&skipped, &[legacy]).is_err());
        let dependent = project::Subtask {
            id: "next".to_string(),
            title: "dependent".to_string(),
            depends_on: vec![skipped.id.clone()],
            ..Default::default()
        };
        assert!(validate_skip_dependencies(&skipped, &[dependent]).is_err());
        let independent = project::Subtask {
            id: "next".to_string(),
            title: "independent".to_string(),
            dependency_notes: "明确不依赖被跳过任务".to_string(),
            ..Default::default()
        };
        assert!(validate_skip_dependencies(&skipped, &[independent]).is_ok());
    }

    #[test]
    fn repeated_signature_stops_before_spending_another_attempt() {
        let mut recovery = project::RecoveryState {
            error_kind: project::RecoveryErrorKind::TestFailure,
            error_signature: "same".to_string(),
            repeated_signature_count: 1,
            attempt: 1,
            max_attempts: 2,
            ..Default::default()
        };
        assert!(!record_failed_signature(
            &mut recovery,
            project::RecoveryErrorKind::TestFailure,
            "same".to_string(),
        ));
        assert_eq!(recovery.repeated_signature_count, 2);
    }

    #[test]
    fn partial_review_is_repairable_only_with_actionable_contract_evidence() {
        let subtask = contract_subtask();
        let authorized = vec!["index.html".to_string()];
        let mut partial = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_evidence_status: project::ReviewEvidenceStatus::Partial,
            review_issues: vec![project::ReviewIssue {
                criterion_index: Some(1),
                criterion: "对象包含 isDefault 字段".to_string(),
                file: "index.html".to_string(),
                expected: "对象包含 isDefault".to_string(),
                actual: "对象缺少 isDefault".to_string(),
                suggested_change: "补充 isDefault".to_string(),
                confidence: 0.9,
                severity: Some(project::ReviewIssueSeverity::Blocking),
                evidence_references: vec![evidence_reference("E001")],
            }],
            ..Default::default()
        };
        assert_eq!(
            classify_test_result_with_context(Some(&partial), Some(&subtask), &authorized),
            project::RecoveryErrorKind::ReviewFailure
        );

        partial.review_issues[0].confidence = 0.6;
        assert_eq!(
            classify_test_result_with_context(Some(&partial), Some(&subtask), &authorized),
            project::RecoveryErrorKind::EvidenceInsufficient
        );
        partial.review_issues[0].confidence = 0.9;
        partial.review_issues[0].file = "outside.html".to_string();
        assert_eq!(
            classify_test_result_with_context(Some(&partial), Some(&subtask), &authorized),
            project::RecoveryErrorKind::EvidenceInsufficient
        );
    }

    #[test]
    fn failure_history_keeps_only_the_latest_entries() {
        let mut recovery = project::RecoveryState::default();
        for failure in ["one", "two", "three", "four", "five"] {
            append_failure_history(&mut recovery, failure);
        }
        assert_eq!(
            recovery.failure_history,
            vec!["two", "three", "four", "five"]
        );
    }

    #[test]
    fn replan_patch_requires_only_patchable_fields() {
        let complete = validate_replan_output(crate::plan_calibration::PlanPatchOutput {
            implementation_guidance: "  完整重执行当前任务  ".to_string(),
            context_summary: "当前代码事实".to_string(),
            evidence_files: vec!["index.html".to_string()],
            dependency_notes: "保留现有依赖契约".to_string(),
            rationale: String::new(),
        })
        .unwrap();
        assert_eq!(
            complete.implementation_guidance.trim(),
            "完整重执行当前任务"
        );

        let missing = validate_replan_output(crate::plan_calibration::PlanPatchOutput {
            implementation_guidance: "任务".to_string(),
            context_summary: String::new(),
            evidence_files: vec![],
            dependency_notes: String::new(),
            rationale: String::new(),
        });
        assert!(missing.is_err());
    }

    #[test]
    fn repair_prompt_uses_the_backend_compiled_contract() {
        let task = project::Subtask {
            title: "拖拽".to_string(),
            goal: "实现拖拽".to_string(),
            execution_prompt: "调用 preventDefault()".to_string(),
            acceptance_criteria: vec!["必须调用 event.preventDefault".to_string()],
            required_identifiers: vec!["event.preventDefault".to_string()],
            evidence_files: vec!["index.html".to_string()],
            ..Default::default()
        };
        let prompt = repair_prompt(
            &project::RecoveryState {
                error_kind: project::RecoveryErrorKind::ReviewFailure,
                ..Default::default()
            },
            &task,
            "criterion 1 failed",
        );
        assert!(prompt.contains("不可变验收标准"));
        assert!(prompt.contains("event.preventDefault"));
        assert!(prompt.contains("index.html"));
    }

    fn healthy_plugin(path: &str) -> crate::engine::EngineHealth {
        crate::engine::EngineHealth {
            runtime: project::ExecutionRuntime::Plugin,
            provider: project::ExecutionProvider::ClaudeCode,
            status: crate::engine::EngineHealthStatus::Available,
            executable_path: Some(path.to_string()),
            version: Some("test".to_string()),
            auth_state: crate::engine::EngineAuthState::Authenticated,
            authentication: crate::engine::EngineAuthenticationResult::unknown("test"),
            supports_unattended: true,
            configuration_valid: true,
            capabilities: vec!["unattended".to_string()],
            source_revision: None,
            runtime_self_test: Default::default(),
            message: "ready".to_string(),
        }
    }

    #[test]
    fn recovery_requires_confirmation_when_settings_or_program_drift() {
        let settings = crate::settings::AppSettings::default();
        let mut session = project::ExecutionSession {
            engine_settings_revision: settings.revision,
            engine_executable_path: "/tmp/claude-a".to_string(),
            ..Default::default()
        };
        assert!(
            execution_snapshot_mismatch(&session, &settings, &healthy_plugin("/tmp/claude-a"),)
                .is_none()
        );

        session.engine_settings_revision = settings.revision.saturating_add(1);
        assert!(
            execution_snapshot_mismatch(&session, &settings, &healthy_plugin("/tmp/claude-a"),)
                .is_some()
        );

        session.engine_settings_revision = settings.revision;
        assert!(
            execution_snapshot_mismatch(&session, &settings, &healthy_plugin("/tmp/claude-b"),)
                .is_some()
        );
    }

    #[test]
    fn legacy_recovery_only_accepts_compatible_default_settings() {
        let session = project::ExecutionSession::default();
        let mut settings = crate::settings::AppSettings::default();
        assert!(
            execution_snapshot_mismatch(&session, &settings, &healthy_plugin("/tmp/claude"),)
                .is_none()
        );
        settings.decision_model.model = "custom-model".to_string();
        assert!(
            execution_snapshot_mismatch(&session, &settings, &healthy_plugin("/tmp/claude"),)
                .is_some()
        );
    }

    #[test]
    fn built_in_recovery_requires_complete_model_and_endpoint_snapshot() {
        let settings = crate::settings::AppSettings::default();
        let source_revision = crate::engine::builtin_grok_source_revision()
            .unwrap_or_else(|| "builtin-grok-disabled-test".to_string());
        let mut profile = project::ExecutionProfile::default();
        profile.runtime = project::ExecutionRuntime::BuiltIn;
        profile.provider = project::ExecutionProvider::GrokBuild;
        let health = crate::engine::EngineHealth {
            runtime: project::ExecutionRuntime::BuiltIn,
            provider: project::ExecutionProvider::GrokBuild,
            status: crate::engine::EngineHealthStatus::Available,
            executable_path: None,
            version: Some("test".to_string()),
            auth_state: crate::engine::EngineAuthState::Authenticated,
            authentication: crate::engine::EngineAuthenticationResult::unknown("test"),
            supports_unattended: true,
            configuration_valid: true,
            capabilities: vec!["embedded".to_string()],
            source_revision: Some(source_revision.clone()),
            runtime_self_test: Default::default(),
            message: "ready".to_string(),
        };
        let mut session = project::ExecutionSession {
            engine_snapshot: profile,
            engine_settings_revision: settings.revision,
            engine_source_revision: source_revision,
            engine_api_backend: settings
                .built_in_grok_build
                .api_backend
                .as_str()
                .to_string(),
            engine_model: settings.built_in_grok_build.model.clone(),
            endpoint_fingerprint: crate::settings::endpoint_fingerprint(
                &settings.built_in_grok_build.api_base_url,
            ),
            ..Default::default()
        };

        assert!(execution_snapshot_mismatch(&session, &settings, &health).is_none());
        session.engine_model.clear();
        assert!(execution_snapshot_mismatch(&session, &settings, &health).is_some());
        session.engine_model = settings.built_in_grok_build.model.clone();
        session.endpoint_fingerprint.clear();
        assert!(execution_snapshot_mismatch(&session, &settings, &health).is_some());
    }
}
