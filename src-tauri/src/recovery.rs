use crate::pipeline::{self, PipelineState, PipelineStatus, SubtaskStatusItem};
use crate::project;
use crate::AppState;
use std::collections::{BTreeMap, BTreeSet};

const MAX_DIAGNOSIS_CHARS: usize = 12_000;
const MAX_EVIDENCE_CHARS: usize = 6_000;
const MAX_FAILURE_HISTORY: usize = 4;
const DEFAULT_MAX_ATTEMPTS: u32 = 2;

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

fn has_review_transport_failure(test: &project::TestResult) -> bool {
    test.warnings
        .iter()
        .any(|warning| warning.contains("AI API") || warning.contains("解析失败"))
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
    match test.automated_test_status {
        project::AutomatedTestStatus::Failed => project::RecoveryErrorKind::TestFailure,
        project::AutomatedTestStatus::Unavailable => project::RecoveryErrorKind::TestUnavailable,
        project::AutomatedTestStatus::Passed
        | project::AutomatedTestStatus::NotConfigured
        | project::AutomatedTestStatus::Unknown => {
            if crate::acceptance::needs_evidence(&test.acceptance_results) {
                return project::RecoveryErrorKind::ValidationFailure;
            }
            if has_review_transport_failure(test)
                || test.review_evidence_status == project::ReviewEvidenceStatus::Unavailable
            {
                return project::RecoveryErrorKind::TestUnavailable;
            }
            match test.review_evidence_status {
                project::ReviewEvidenceStatus::Complete => {
                    if subtask.is_some_and(|subtask| {
                        !actionable_recovery_issues(test, subtask, authorized_paths).is_empty()
                    }) {
                        project::RecoveryErrorKind::ReviewFailure
                    } else {
                        project::RecoveryErrorKind::ValidationFailure
                    }
                }
                project::ReviewEvidenceStatus::Partial
                    if subtask.is_some_and(|subtask| {
                        !actionable_recovery_issues(test, subtask, authorized_paths).is_empty()
                    }) =>
                {
                    project::RecoveryErrorKind::ReviewFailure
                }
                project::ReviewEvidenceStatus::Partial
                | project::ReviewEvidenceStatus::Unavailable => {
                    project::RecoveryErrorKind::ValidationFailure
                }
            }
        }
    }
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
    pipeline::write_execution_history(
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
    let automatic = !matches!(
        kind,
        project::RecoveryErrorKind::TestUnavailable | project::RecoveryErrorKind::ValidationFailure
    );
    if !automatic {
        recovery.phase = project::RecoveryPhase::WaitingHuman;
    }
    proj.workflow_state.recovery_state = Some(recovery);
    pipeline::write_execution_history(
        proj,
        "error",
        project::ExecutionEventType::RecoveryStarted,
        format!("质量错误已分类：{:?}", kind),
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );
    if automatic {
        set_autopilot_recovering(proj, "正在诊断质量错误");
    } else {
        set_autopilot_waiting(proj, "验收证据不可用或不足，需要重建证据后再判断");
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
    pipeline::write_execution_history(
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
    );
    truncate_chars(
        &format!(
            "恢复类型：{:?}\n当前目标：{}\n验收标准（最高优先级，精确标识符必须逐字遵循）：\n- {}\n当前未满足项：\n{}\n修复轮次历史：\n{}{}\n匹配的纠错经验：\n{}\n允许修改：\n- {}\n允许新建：\n- {}\n当前基线：{}\n失败证据：\n{}\n执行错误：\n{}\n当前受限 diff：\n{}\n上次修复摘要：\n{}",
            recovery.error_kind,
            if subtask.goal.is_empty() { &subtask.title } else { &subtask.goal },
            subtask.acceptance_criteria.join("\n- "),
            issue_list_for_prompt(&recovery.active_issues),
            attempt_history_for_prompt(&recovery.attempt_history),
            strategy_note,
            if learning.is_empty() { "（无）" } else { &learning },
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

    let current_facts = crate::project_facts::capture(
        &proj.project_path,
        &crate::project_facts::snapshot_paths(subtask),
        crate::project_facts::accepted_deviations(proj),
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
    pipeline::write_execution_history(
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
        stdout: repair.stdout,
        stderr: repair.stderr,
        engine_failure_kind: repair.engine_failure_kind,
    }
}

#[tauri::command]
pub(crate) async fn run_error_recovery(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let mut pipeline_guard = state.pipeline_state.lock().await;
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
    if recovery.phase == project::RecoveryPhase::Replanning {
        match replan_current_subtask(&mut proj, &recovery, &session, &subtask, &authorized_paths)
            .await
        {
            Ok(()) => {}
            Err(error) => {
                mark_waiting_human(&mut proj, project::RecoveryErrorKind::HumanRequired, &error);
                pipeline::write_execution_history(
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

    pipeline::write_execution_history(
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
        pipeline::write_execution_history(
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
    pipeline::write_execution_history(
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
        &session.engine_snapshot,
        crate::engine::ExecutionRequest {
            project_path: proj.project_path.clone(),
            prompt,
            authorized_paths: authorized_paths.clone(),
            subtask_id: session.subtask_id.clone(),
            execution_id: recovery_execution_id.clone(),
        },
        state.pipeline_state.clone(),
    )
    .await;

    let mut pipeline_guard = state.pipeline_state.lock().await;
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
        pipeline::write_execution_history(
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
        .and_then(|item| item.execution_result.clone());
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
        item.execution_result = Some(merged_execution);
    }
    pipeline::write_execution_history(
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

    let original_prompt = if subtask.execution_prompt.is_empty() {
        subtask.prompt.clone()
    } else {
        subtask.execution_prompt.clone()
    };
    let test = crate::test_runner::check_subtask_with_context(
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
        Some(original_prompt),
    )
    .await
    .unwrap_or(project::TestResult {
        passed: false,
        issues: vec!["测试服务不可用".to_string()],
        suggestion: "请人工核验".to_string(),
        automated_test_status: project::AutomatedTestStatus::Unavailable,
        ..Default::default()
    });

    let mut pipeline_guard = state.pipeline_state.lock().await;
    let mut proj = crate::load_project(&project_name)?;
    let still_current = proj
        .workflow_state
        .recovery_state
        .as_ref()
        .is_some_and(|current| current.execution_id == recovery_execution_id)
        && proj.execution_session.as_ref().is_some_and(|current| {
            current.active
                && current.status.eq_ignore_ascii_case("recovering")
                && current.execution_id == recovery_execution_id
        });
    if !still_current {
        if !recovery.checkpoint_id.is_empty() {
            let _ = crate::recovery_checkpoint::discard(&recovery.checkpoint_id);
        }
        return Ok(proj);
    }
    finish_retest(&mut proj, &session, &recovery_execution_id, test.clone())?;
    set_pipeline_terminal(
        &mut pipeline_guard,
        &recovery_execution_id,
        Some(test),
        None,
    );
    crate::save_project(&proj)?;
    crate::load_project(&project_name)
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
    pipeline::write_execution_history(
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
        pipeline::write_execution_history(
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
) -> Result<(), String> {
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
    pipeline::write_execution_history(
        proj,
        if test.passed { "success" } else { "error" },
        project::ExecutionEventType::RetestCompleted,
        if test.passed {
            "自动修复复测通过".to_string()
        } else {
            format!("自动修复复测未通过：{}", truncate_chars(&summary, 1_000))
        },
        Some(&session.milestone_id),
        Some(&session.mid_stage_id),
        Some(&session.subtask_id),
    );

    if test.passed {
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
        pipeline::write_execution_history(
            proj,
            "success",
            project::ExecutionEventType::RecoverySucceeded,
            "自动修复成功，恢复正常自动驾驶".to_string(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
        if let Some(completed_recovery) = proj.workflow_state.recovery_state.clone() {
            let strategy = if completed_recovery.replan_attempted {
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
            autopilot.last_action = "自动修复成功，继续执行".to_string();
            autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
            autopilot.error_message.clear();
            autopilot.recovery_action = project::AutopilotRecoveryAction::None;
        }
        touch(proj);
        return Ok(());
    }

    let mut next_kind =
        classify_test_result_with_context(Some(&test), Some(&subtask), &authorized_paths);
    let next_issues = recovery_issues(&test, &subtask, &authorized_paths);
    let changed_files = subtask
        .execution_result
        .as_ref()
        .map(|result| result.file_changes.clone())
        .unwrap_or_default();
    let mut next_phase = project::RecoveryPhase::Diagnosing;
    let mut contradictory_criteria = Vec::new();
    if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
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
            next_kind = project::RecoveryErrorKind::ValidationFailure;
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
        recovery.original_test_failure = truncate_chars(&summary, 4_000);
        append_failure_history(recovery, &summary);
        recovery.active_issues = next_issues;
        recovery.last_repair_summary = attempt_summary;
        recovery.updated_at = chrono::Utc::now().to_rfc3339();
        let checkpoint_id = recovery.checkpoint_id.clone();
        recovery.checkpoint_id.clear();
        if introduced_regression && !checkpoint_id.is_empty() {
            crate::recovery_checkpoint::restore(&checkpoint_id)?;
            recovery.last_repair_summary = format!(
                "{}；检测到 {} 个新增回归，已撤销本轮修复",
                recovery.last_repair_summary, regression_count
            );
        } else if !checkpoint_id.is_empty() {
            crate::recovery_checkpoint::discard(&checkpoint_id)?;
        }
        let next_signature = normalized_signature(&next_kind, &summary);
        let regular_repair_exhausted =
            record_failed_signature(recovery, next_kind.clone(), next_signature);
        next_phase = if matches!(
            next_kind,
            project::RecoveryErrorKind::TestUnavailable
                | project::RecoveryErrorKind::ValidationFailure
        ) {
            project::RecoveryPhase::WaitingHuman
        } else if recovery.replan_execution_attempted {
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
        set_autopilot_waiting(proj, "自动修复未能通过复测，等待人工处理");
        pipeline::write_execution_history(
            proj,
            "error",
            project::ExecutionEventType::RecoveryExhausted,
            "自动修复达到停止条件，等待人工处理".to_string(),
            Some(&session.milestone_id),
            Some(&session.mid_stage_id),
            Some(&session.subtask_id),
        );
    } else if next_phase == project::RecoveryPhase::Replanning {
        set_autopilot_recovering(proj, "常规修复耗尽，正在重新规划当前任务");
        pipeline::write_execution_history(
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
    Ok(())
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
    let (recovery, session, subtask) = current_recovery_context(&proj)?;
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
            pipeline::write_execution_history(
                &mut proj,
                "success",
                project::ExecutionEventType::HumanVerificationAccepted,
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
            pipeline::write_execution_history(
                &mut proj,
                "pause",
                project::ExecutionEventType::TaskSkipped,
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
                pipeline::write_execution_history(
                    &mut proj,
                    "success",
                    project::ExecutionEventType::MidStageComplete,
                    "中阶段所有任务已达到终态".to_string(),
                    Some(&session.milestone_id),
                    Some(&session.mid_stage_id),
                    None,
                );
            }
            if milestone_completed {
                pipeline::write_execution_history(
                    &mut proj,
                    "success",
                    project::ExecutionEventType::AdvanceMilestoneReview,
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
            pipeline::write_execution_history(
                &mut proj,
                "info",
                project::ExecutionEventType::ReplanStarted,
                "人工请求当前小阶段受限重规划".to_string(),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
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
            let test = crate::test_runner::check_subtask_with_context(
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
            )
            .await
            .unwrap_or(project::TestResult {
                passed: false,
                issues: vec!["测试服务不可用".to_string()],
                suggestion: "请人工核验".to_string(),
                automated_test_status: project::AutomatedTestStatus::Unavailable,
                ..Default::default()
            });
            let mut proj = crate::load_project(&project_name)?;
            if test.passed {
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
                pipeline::write_execution_history(
                    &mut proj,
                    "success",
                    project::ExecutionEventType::RecoverySucceeded,
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
            return crate::load_project(&project_name);
        }
        _ => return Err(format!("未知的人工恢复动作：{}", resolution)),
    }

    touch(&mut proj);
    crate::save_project(&proj)?;
    crate::load_project(&project_name)
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
            project::RecoveryErrorKind::TestUnavailable
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

        let partial_review = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_evidence_status: project::ReviewEvidenceStatus::Partial,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&partial_review)),
            project::RecoveryErrorKind::ValidationFailure
        );

        let complete_review = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_evidence_status: project::ReviewEvidenceStatus::Complete,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&complete_review)),
            project::RecoveryErrorKind::ValidationFailure
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
            project::RecoveryErrorKind::ValidationFailure
        );
        partial.review_issues[0].confidence = 0.9;
        partial.review_issues[0].file = "outside.html".to_string();
        assert_eq!(
            classify_test_result_with_context(Some(&partial), Some(&subtask), &authorized),
            project::RecoveryErrorKind::ValidationFailure
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
}
