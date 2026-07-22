use crate::pipeline::{self, PipelineState, PipelineStatus, SubtaskStatusItem};
use crate::project;
use crate::AppState;
use std::collections::BTreeSet;

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

pub(crate) fn classify_test_result(
    test: Option<&project::TestResult>,
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
            if test.review_evidence_status != project::ReviewEvidenceStatus::Complete
                || test
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("AI API") || warning.contains("解析失败"))
            {
                project::RecoveryErrorKind::TestUnavailable
            } else {
                project::RecoveryErrorKind::ReviewFailure
            }
        }
    }
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
        started_at: now.clone(),
        updated_at: now,
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
        classify_test_result(subtask.test_result.as_ref())
    };
    let failure = test_failure_summary(subtask.test_result.as_ref(), gate_reason);
    let mut recovery = create_recovery_state(
        kind.clone(),
        subtask.id.clone(),
        session.execution_id.clone(),
        session.base_commit.clone(),
        truncate_chars(&failure, 4_000),
    );
    let automatic = !matches!(kind, project::RecoveryErrorKind::TestUnavailable);
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
        set_autopilot_waiting(proj, "测试或审查服务不可用，需要人工核验");
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
    truncate_chars(
        &format!(
            "恢复类型：{:?}\n当前目标：{}\n验收标准：\n- {}\n允许修改：\n- {}\n允许新建：\n- {}\n当前基线：{}\n失败证据：\n{}\n执行错误：\n{}\n当前受限 diff：\n{}\n上次修复摘要：\n{}",
            recovery.error_kind,
            if subtask.goal.is_empty() { &subtask.title } else { &subtask.goal },
            subtask.acceptance_criteria.join("\n- "),
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
    let original = if subtask.execution_prompt.is_empty() {
        &subtask.prompt
    } else {
        &subtask.execution_prompt
    };
    if recovery.error_kind == project::RecoveryErrorKind::ExecutionError {
        format!(
            "重新执行已批准的当前小阶段。上次执行器异常，已恢复到执行基线。不得扩大任务范围。\n\n原始任务：\n{}\n\n异常摘要：\n{}",
            original, diagnosis
        )
    } else {
        format!(
            "只修复当前小阶段的已知失败，不重新设计、不扩展任务范围。完成修复后直接结束。\n\n原始任务：\n{}\n\n受限诊断上下文：\n{}",
            original, diagnosis
        )
    }
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
    if recovery.phase == project::RecoveryPhase::WaitingHuman {
        return Err("自动恢复已停止，等待人工处理。".to_string());
    }
    if recovery.attempt >= recovery.max_attempts {
        mark_waiting_human(&mut proj, recovery.error_kind, "自动修复次数已用尽");
        crate::save_project(&proj)?;
        return crate::load_project(&project_name);
    }

    let authorized_paths = crate::plan_contract::validate_subtask(&subtask, "错误恢复任务")?;
    let diagnosis = build_diagnosis(&proj, &recovery, &subtask, &authorized_paths);
    recovery.attempt = recovery.attempt.saturating_add(1);
    recovery.phase = project::RecoveryPhase::Repairing;
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
        project::ExecutionEventType::RepairAttemptStarted,
        format!(
            "开始第 {}/{} 次自动修复（{}）",
            recovery.attempt,
            recovery.max_attempts,
            session.engine_snapshot.provider.display_name(),
        ),
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
        return crate::load_project(&project_name);
    }

    let repair_result = match repair_result {
        Ok(result) if result.success => result,
        Ok(result) => {
            let message = if result.error_log.is_empty() {
                format!(
                    "{} 修复进程非零退出",
                    session.engine_snapshot.provider.display_name()
                )
            } else {
                result.error_log
            };
            handle_repair_execution_failure(
                &mut proj,
                &session,
                &recovery_execution_id,
                &message,
                &mut pipeline_guard,
            )?;
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        Err(crate::engine::EngineError::Cancelled) => {
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
            handle_repair_execution_failure(
                &mut proj,
                &session,
                &recovery_execution_id,
                "自动修复执行超时",
                &mut pipeline_guard,
            )?;
            crate::save_project(&proj)?;
            return crate::load_project(&project_name);
        }
        Err(error) => {
            handle_repair_execution_failure(
                &mut proj,
                &session,
                &recovery_execution_id,
                &error.to_string(),
                &mut pipeline_guard,
            )?;
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
    if restore_result.is_err() || attempt >= max_attempts {
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
    test: project::TestResult,
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

    let next_kind = classify_test_result(Some(&test));
    let next_signature = normalized_signature(&next_kind, &summary);
    let mut should_wait = next_kind == project::RecoveryErrorKind::TestUnavailable;
    if let Some(recovery) = proj.workflow_state.recovery_state.as_mut() {
        recovery.original_test_failure = truncate_chars(&summary, 4_000);
        recovery.updated_at = chrono::Utc::now().to_rfc3339();
        should_wait =
            should_wait || record_failed_signature(recovery, next_kind.clone(), next_signature);
        recovery.phase = if should_wait {
            project::RecoveryPhase::WaitingHuman
        } else {
            project::RecoveryPhase::Diagnosing
        };
    }

    if let Some(current_session) = proj.execution_session.as_mut() {
        current_session.execution_id = execution_id.to_string();
        current_session.active = true;
        current_session.status = if should_wait {
            "quality_blocked".to_string()
        } else {
            "awaiting_confirmation".to_string()
        };
        current_session.failure_message = truncate_chars(&summary, 2_048);
        current_session.state_entered_at = chrono::Utc::now().to_rfc3339();
    }

    if should_wait {
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

#[tauri::command]
pub(crate) async fn resolve_human_recovery(
    state: tauri::State<'_, AppState>,
    project_name: String,
    resolution: String,
    reason: String,
) -> Result<project::Project, String> {
    let _pipeline_guard = state.pipeline_state.lock().await;
    let mut proj = crate::load_project(&project_name)?;
    let (_recovery, session, subtask) = current_recovery_context(&proj)?;
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
        "human_override" => {
            if reason.trim().is_empty() {
                return Err("人工核验通过必须填写原因。".to_string());
            }
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
            item.human_verification = Some(project::HumanVerification {
                verification_kind: project::VerificationKind::HumanOverride,
                verification_reason: reason.clone(),
                verified_at: chrono::Utc::now().to_rfc3339(),
                original_test_failure: original_failure,
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
                format!("人工核验通过：{}", reason.trim()),
                Some(&session.milestone_id),
                Some(&session.mid_stage_id),
                Some(&session.subtask_id),
            );
            proj.workflow_state.recovery_state = None;
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::Running;
                autopilot.last_action = "人工核验已记录，继续执行".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                autopilot.error_message.clear();
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            }
        }
        "regenerate_plan" => {
            let current_mid = proj
                .milestones
                .iter()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter()
                        .find(|mid_stage| mid_stage.id == session.mid_stage_id)
                })
                .ok_or_else(|| "无法定位要重新规划的中阶段。".to_string())?;
            if current_mid
                .subtasks
                .iter()
                .any(|item| item.status == project::SubtaskStatus::Passed)
            {
                return Err(
                    "当前中阶段已有通过的小阶段，不能直接替换整个执行计划；请使用回退流程选择稳定点。"
                        .to_string(),
                );
            }
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
            let current_mid = proj
                .milestones
                .iter_mut()
                .find(|milestone| milestone.id == session.milestone_id)
                .and_then(|milestone| {
                    milestone
                        .mid_stages
                        .iter_mut()
                        .find(|mid_stage| mid_stage.id == session.mid_stage_id)
                })
                .ok_or_else(|| "无法定位要重新规划的中阶段。".to_string())?;
            current_mid.subtasks.clear();
            current_mid.plan_approved_at = None;
            current_mid.plan_revision = 0;
            current_mid.plan_check_result = None;
            current_mid.plan_generated_at = None;
            current_mid.status = project::MidStageStatus::Ready;
            proj.execution_session = None;
            proj.workflow_state.recovery_state = None;
            proj.workflow_state.current_step = project::WorkflowStep::PlanGeneration;
            if let Some(autopilot) = proj.workflow_state.autopilot_state.as_mut() {
                autopilot.run_status = project::AutopilotRunStatus::Running;
                autopilot.last_action = "已回到当前执行计划生成步骤".to_string();
                autopilot.last_action_at = chrono::Utc::now().to_rfc3339();
                autopilot.error_message.clear();
                autopilot.recovery_action = project::AutopilotRecoveryAction::None;
            }
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
                    classify_test_result(Some(&test)),
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
            project::RecoveryErrorKind::TestUnavailable
        );

        let complete_review = project::TestResult {
            passed: false,
            automated_test_status: project::AutomatedTestStatus::Passed,
            review_evidence_status: project::ReviewEvidenceStatus::Complete,
            ..Default::default()
        };
        assert_eq!(
            classify_test_result(Some(&complete_review)),
            project::RecoveryErrorKind::ReviewFailure
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
        assert!(record_failed_signature(
            &mut recovery,
            project::RecoveryErrorKind::TestFailure,
            "same".to_string(),
        ));
        assert_eq!(recovery.repeated_signature_count, 2);
    }
}
