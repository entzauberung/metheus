use crate::project::{
    AutopilotRecoveryAction, GitConfirmationFailureKind, Project, RecoveryErrorKind, RecoveryPhase,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RECOVERY_PRESENTATION_VERSION: &str = "recovery-presentation-v5";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoveryPresentationKind {
    None,
    ControlActionOccupied,
    BaselineRecovery,
    GitReconfirmation,
    EngineBlocked,
    ValidationRetry,
    EvidenceInsufficient,
    HumanDecision,
    AutomaticRecovery,
    RetryAdvance,
    RegeneratePlan,
    PrepareWorkspace,
    ResolveWorkspaceChanges,
    SyncAndClose,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoverySeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryProgressStatus {
    #[default]
    Inactive,
    Queued,
    Scheduled,
    Running,
    Warning,
    Stalled,
    WaitingHuman,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoveryCapability {
    SyncProject,
    ClearStaleControlLock,
    AcknowledgeExecutionRecovery,
    RetryGitConfirmation,
    RetryAutopilotAdvance,
    RegenerateExecutionPlan,
    PrepareExecutionWorkspace,
    RefreshExecutionWorkspace,
    RunAutomaticRecovery,
    ResolveHumanRecovery,
    ResumeAutopilot,
    CloseAutopilot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryActionPresentation {
    pub capability: RecoveryCapability,
    pub label: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryDecisionResolution {
    Retest,
    Revalidate,
    RestoreAndRetry,
    RegeneratePlan,
    ConfirmActualPass,
    AcceptDeviation,
    SkipTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryDecisionOption {
    pub resolution: RecoveryDecisionResolution,
    pub label: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub requires_reason: bool,
    pub requires_acceptance_selection: bool,
    pub requires_baseline_preview: bool,
    pub preview_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryPresentation {
    pub presentation_version: String,
    pub kind: RecoveryPresentationKind,
    pub title: String,
    pub reason: String,
    pub severity: RecoverySeverity,
    pub primary_action: Option<RecoveryActionPresentation>,
    pub secondary_actions: Vec<RecoveryActionPresentation>,
    pub preserve_current_code: bool,
    pub requires_baseline_restore: bool,
    pub supports_preview: bool,
    pub automatic_retry: bool,
    pub capabilities: Vec<RecoveryCapability>,
    pub decision_options: Vec<RecoveryDecisionOption>,
    pub state_fingerprint: String,
    pub phase_label: String,
    pub background_retry_active: bool,
    pub background_retry_summary: String,
    #[serde(default)]
    pub progress_status: RecoveryProgressStatus,
    #[serde(default)]
    pub current_action: Option<String>,
    #[serde(default)]
    pub action_started_at: Option<String>,
    #[serde(default)]
    pub last_progress_at: Option<String>,
    #[serde(default)]
    pub elapsed_seconds: Option<u64>,
    #[serde(default)]
    pub warning_at: Option<String>,
    #[serde(default)]
    pub hard_deadline_at: Option<String>,
    pub post_action_expectation: String,
    pub stale_risk: bool,
    pub sync_risk_summary: String,
    pub sync_needed: bool,
    pub code_impact_summary: String,
    pub affected_task_label: String,
    pub baseline_reference: String,
    pub validation_phase_label: String,
    pub retry_count: u32,
    pub retry_limit: u32,
    pub next_retry_at: Option<String>,
    pub validation_retry_count: u32,
    pub validation_retry_limit: u32,
    pub next_validation_retry_at: Option<String>,
    pub heartbeat_status: String,
    pub automated_test_status: String,
    pub code_review_status: String,
    pub review_protocol_status: String,
    pub acceptance_evidence_status: String,
    pub control_lock_valid: Option<bool>,
    pub control_action_description: String,
    pub control_action_elapsed_seconds: u64,
    pub control_lock_last_heartbeat_at: Option<String>,
    pub control_lock_failure_reason: String,
    pub control_lock_cleanup_available: bool,
}

impl RecoveryPresentation {
    fn none(project: &Project) -> Self {
        Self {
            presentation_version: RECOVERY_PRESENTATION_VERSION.to_string(),
            kind: RecoveryPresentationKind::None,
            title: String::new(),
            reason: String::new(),
            severity: RecoverySeverity::Info,
            primary_action: None,
            secondary_actions: Vec::new(),
            preserve_current_code: true,
            requires_baseline_restore: false,
            supports_preview: false,
            automatic_retry: false,
            capabilities: Vec::new(),
            decision_options: Vec::new(),
            state_fingerprint: fingerprint(project, &RecoveryPresentationKind::None),
            phase_label: String::new(),
            background_retry_active: false,
            background_retry_summary: String::new(),
            progress_status: RecoveryProgressStatus::Inactive,
            current_action: None,
            action_started_at: None,
            last_progress_at: None,
            elapsed_seconds: None,
            warning_at: None,
            hard_deadline_at: None,
            post_action_expectation: String::new(),
            stale_risk: false,
            sync_risk_summary: String::new(),
            sync_needed: false,
            code_impact_summary: String::new(),
            affected_task_label: String::new(),
            baseline_reference: String::new(),
            validation_phase_label: String::new(),
            retry_count: 0,
            retry_limit: crate::autopilot_failure::MAX_TRANSIENT_RETRIES,
            next_retry_at: None,
            validation_retry_count: 0,
            validation_retry_limit: 0,
            next_validation_retry_at: None,
            heartbeat_status: String::new(),
            automated_test_status: String::new(),
            code_review_status: String::new(),
            review_protocol_status: String::new(),
            acceptance_evidence_status: String::new(),
            control_lock_valid: None,
            control_action_description: String::new(),
            control_action_elapsed_seconds: 0,
            control_lock_last_heartbeat_at: None,
            control_lock_failure_reason: String::new(),
            control_lock_cleanup_available: false,
        }
    }
}

fn action(capability: RecoveryCapability, label: &str) -> RecoveryActionPresentation {
    RecoveryActionPresentation {
        capability,
        label: label.to_string(),
        enabled: true,
        disabled_reason: None,
    }
}

fn disabled_action(
    capability: RecoveryCapability,
    label: &str,
    reason: &str,
) -> RecoveryActionPresentation {
    RecoveryActionPresentation {
        capability,
        label: label.to_string(),
        enabled: false,
        disabled_reason: Some(reason.to_string()),
    }
}

fn decision(
    resolution: RecoveryDecisionResolution,
    label: &str,
    requires_reason: bool,
    requires_acceptance_selection: bool,
    requires_baseline_preview: bool,
) -> RecoveryDecisionOption {
    RecoveryDecisionOption {
        resolution,
        label: label.to_string(),
        enabled: true,
        disabled_reason: None,
        requires_reason,
        requires_acceptance_selection,
        requires_baseline_preview,
        preview_message: if requires_baseline_preview {
            "执行前会核对最新工作区影响。".to_string()
        } else {
            String::new()
        },
    }
}

fn disabled_decision(
    resolution: RecoveryDecisionResolution,
    label: &str,
    reason: &str,
    requires_reason: bool,
    requires_acceptance_selection: bool,
    requires_baseline_preview: bool,
) -> RecoveryDecisionOption {
    RecoveryDecisionOption {
        resolution,
        label: label.to_string(),
        enabled: false,
        disabled_reason: Some(reason.to_string()),
        requires_reason,
        requires_acceptance_selection,
        requires_baseline_preview,
        preview_message: if requires_baseline_preview {
            "执行前会核对最新工作区影响。".to_string()
        } else {
            String::new()
        },
    }
}

fn reason(project: &Project, fallback: &str) -> String {
    project
        .execution_session
        .as_ref()
        .map(|session| session.failure_message.trim())
        .filter(|message| !message.is_empty())
        .or_else(|| {
            project
                .workflow_state
                .recovery_state
                .as_ref()
                .map(|recovery| recovery.last_diagnosis.trim())
                .filter(|message| !message.is_empty())
        })
        .or_else(|| {
            project
                .workflow_state
                .autopilot_state
                .as_ref()
                .map(|autopilot| autopilot.error_message.trim())
                .filter(|message| !message.is_empty())
        })
        .unwrap_or(fallback)
        .to_string()
}

fn retryable_git_confirmation(failure: Option<&GitConfirmationFailureKind>) -> bool {
    matches!(
        failure,
        Some(
            GitConfirmationFailureKind::LegacyV1TagConflict
                | GitConfirmationFailureKind::CommitFailed
                | GitConfirmationFailureKind::TagFailed
                | GitConfirmationFailureKind::ProjectFinalizationFailed
                | GitConfirmationFailureKind::GitMetadataUnavailable
        )
    )
}

fn git_reconfirmation_reason(project: &Project) -> String {
    const PRESERVATION_NOTICE: &str = "代码与质量结果已保留";
    let base = reason(project, "请核对 Git 确认事务。");
    if base.contains(PRESERVATION_NOTICE) {
        base
    } else {
        format!(
            "{}；{}。",
            base.trim_end_matches(['。', '；']),
            PRESERVATION_NOTICE
        )
    }
}

fn validation_recovery(kind: &RecoveryErrorKind) -> bool {
    matches!(
        kind,
        RecoveryErrorKind::ReviewTransientFailure
            | RecoveryErrorKind::ReviewProtocolFailure
            | RecoveryErrorKind::ReviewServiceBlocked
    )
}

fn fingerprint(project: &Project, kind: &RecoveryPresentationKind) -> String {
    let session = project.execution_session.as_ref();
    let recovery = project.workflow_state.recovery_state.as_ref();
    let autopilot = project.workflow_state.autopilot_state.as_ref();
    let stable_state = format!(
        "{}|{:?}|{}|{}|{:?}|{:?}|{}|{:?}|{}|{}",
        project.name,
        kind,
        session
            .map(|value| value.execution_id.as_str())
            .unwrap_or(""),
        session.map(|value| value.status.as_str()).unwrap_or(""),
        session.and_then(|value| value.confirmation_failure_kind.as_ref()),
        recovery.map(|value| (&value.error_kind, &value.phase)),
        recovery
            .map(|value| value.error_signature.as_str())
            .unwrap_or(""),
        autopilot.map(|value| &value.recovery_action),
        project.task_control.active_action_id,
        project
            .task_control
            .active_action_lease
            .as_ref()
            .map(|lease| lease.heartbeat_at.as_str())
            .unwrap_or(""),
    );
    let digest = Sha256::digest(stable_state.as_bytes());
    format!("{:x}", digest)
}

struct RecoveryProgressPresentation {
    status: RecoveryProgressStatus,
    current_action: Option<String>,
    action_started_at: Option<String>,
    last_progress_at: Option<String>,
    elapsed_seconds: Option<u64>,
    warning_at: Option<String>,
    hard_deadline_at: Option<String>,
}

fn recovery_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
}

fn recovery_progress_at(
    project: &Project,
    now: chrono::DateTime<chrono::Utc>,
    automatic_retry: bool,
) -> RecoveryProgressPresentation {
    let recovery = project.workflow_state.recovery_state.as_ref();
    let autopilot = project.workflow_state.autopilot_state.as_ref();
    let waiting_human = recovery
        .map(|state| state.phase == RecoveryPhase::WaitingHuman)
        .unwrap_or(false)
        || autopilot
            .map(|state| {
                state.run_status == crate::project::AutopilotRunStatus::ErrorStopped
                    || state.recovery_action == AutopilotRecoveryAction::WaitHumanDecision
            })
            .unwrap_or(false);
    let claimed = autopilot.filter(|state| {
        !waiting_human
            && !state.current_action_id.is_empty()
            && state.current_action_kind == "run_error_recovery"
    });
    let action_started_at = claimed
        .map(|state| state.action_started_at.clone())
        .filter(|value| !value.is_empty());
    let action_started = action_started_at.as_deref().and_then(recovery_timestamp);
    let last_progress_at = recovery
        .map(|state| state.updated_at.clone())
        .filter(|value| !value.is_empty());
    let last_progress = last_progress_at.as_deref().and_then(recovery_timestamp);
    let scheduled = !waiting_human
        && autopilot
            .and_then(|state| state.next_retry_at.as_deref())
            .and_then(recovery_timestamp)
            .into_iter()
            .chain(
                recovery
                    .and_then(|state| state.next_validation_retry_at.as_deref())
                    .and_then(recovery_timestamp),
            )
            .any(|deadline| deadline > now);

    let status = if waiting_human {
        RecoveryProgressStatus::WaitingHuman
    } else if claimed.is_some() {
        let idle_seconds = last_progress
            .map(|progress| now.signed_duration_since(progress).num_seconds().max(0))
            .unwrap_or(0);
        if idle_seconds >= crate::autopilot_runtime::RECOVERY_PROGRESS_STALLED_SECS {
            RecoveryProgressStatus::Stalled
        } else if idle_seconds >= crate::autopilot_runtime::RECOVERY_PROGRESS_WARNING_SECS {
            RecoveryProgressStatus::Warning
        } else {
            RecoveryProgressStatus::Running
        }
    } else if scheduled {
        RecoveryProgressStatus::Scheduled
    } else if automatic_retry && recovery.is_some() {
        RecoveryProgressStatus::Queued
    } else {
        RecoveryProgressStatus::Inactive
    };

    RecoveryProgressPresentation {
        status,
        current_action: claimed.map(|state| state.current_action_kind.clone()),
        action_started_at,
        last_progress_at,
        elapsed_seconds: action_started
            .map(|started| now.signed_duration_since(started).num_seconds().max(0) as u64),
        warning_at: last_progress.map(|progress| {
            (progress
                + chrono::Duration::seconds(
                    crate::autopilot_runtime::RECOVERY_PROGRESS_WARNING_SECS as i64,
                ))
            .to_rfc3339()
        }),
        hard_deadline_at: action_started.map(|started| {
            (started
                + chrono::Duration::seconds(
                    crate::autopilot_runtime::RECOVERY_ACTION_HARD_TIMEOUT_SECS as i64,
                ))
            .to_rfc3339()
        }),
    }
}

fn finish(
    project: &Project,
    kind: RecoveryPresentationKind,
    title: &str,
    reason_text: String,
    severity: RecoverySeverity,
    primary_action: Option<RecoveryActionPresentation>,
    mut secondary_actions: Vec<RecoveryActionPresentation>,
    preserve_current_code: bool,
    requires_baseline_restore: bool,
    supports_preview: bool,
    automatic_retry: bool,
    decision_options: Vec<RecoveryDecisionOption>,
) -> RecoveryPresentation {
    if kind != RecoveryPresentationKind::None
        && kind != RecoveryPresentationKind::ControlActionOccupied
    {
        secondary_actions.insert(0, action(RecoveryCapability::SyncProject, "同步状态"));
    }
    let mut capabilities = primary_action
        .iter()
        .map(|item| item.capability.clone())
        .chain(secondary_actions.iter().map(|item| item.capability.clone()))
        .collect::<Vec<_>>();
    capabilities.dedup();
    let state_fingerprint = fingerprint(project, &kind);
    let phase_label = recovery_phase_label(project, &kind);
    let post_action_expectation = post_action_expectation(&kind, primary_action.as_ref());
    let sync_needed = kind == RecoveryPresentationKind::SyncAndClose;
    let sync_risk_summary = if sync_needed {
        "最终状态可能延迟，请等待统一同步。".to_string()
    } else {
        String::new()
    };
    let recovery = project.workflow_state.recovery_state.as_ref();
    let autopilot = project.workflow_state.autopilot_state.as_ref();
    let task = recovery_task(project);
    let test = task.and_then(|value| value.test_result.as_ref());
    let ledger = task
        .map(|value| value.acceptance_ledger.as_slice())
        .unwrap_or_default();
    let (automated_test_status, code_review_status, review_protocol_status) =
        quality_statuses(test);
    let progress = recovery_progress_at(project, chrono::Utc::now(), automatic_retry);
    let background_retry_active = matches!(
        progress.status,
        RecoveryProgressStatus::Scheduled
            | RecoveryProgressStatus::Running
            | RecoveryProgressStatus::Warning
            | RecoveryProgressStatus::Stalled
    );
    let background_retry_summary = match progress.status {
        RecoveryProgressStatus::Scheduled => "后台重试已安排",
        RecoveryProgressStatus::Running => "后台恢复动作进行中",
        RecoveryProgressStatus::Warning => "后台恢复仍在运行，但业务进展延迟",
        RecoveryProgressStatus::Stalled => "后台恢复动作已停滞，将在超时边界停止",
        RecoveryProgressStatus::Inactive
        | RecoveryProgressStatus::Queued
        | RecoveryProgressStatus::WaitingHuman => "",
    }
    .to_string();
    let code_impact_summary = baseline_impact_summary(project);
    RecoveryPresentation {
        presentation_version: RECOVERY_PRESENTATION_VERSION.to_string(),
        kind,
        title: title.to_string(),
        reason: reason_text,
        severity,
        primary_action,
        secondary_actions,
        preserve_current_code,
        requires_baseline_restore,
        supports_preview,
        automatic_retry,
        capabilities,
        decision_options,
        state_fingerprint,
        phase_label,
        background_retry_active,
        background_retry_summary,
        progress_status: progress.status,
        current_action: progress.current_action,
        action_started_at: progress.action_started_at,
        last_progress_at: progress.last_progress_at,
        elapsed_seconds: progress.elapsed_seconds,
        warning_at: progress.warning_at,
        hard_deadline_at: progress.hard_deadline_at,
        post_action_expectation,
        stale_risk: sync_needed,
        sync_risk_summary,
        sync_needed,
        code_impact_summary,
        affected_task_label: affected_task_label(project, task),
        baseline_reference: baseline_reference(project),
        validation_phase_label: validation_phase_label(project, test),
        retry_count: autopilot
            .map(|state| state.transient_retry_count)
            .unwrap_or(0),
        retry_limit: crate::autopilot_failure::MAX_TRANSIENT_RETRIES,
        next_retry_at: autopilot.and_then(|state| state.next_retry_at.clone()),
        validation_retry_count: recovery
            .map(|state| state.validation_retry_count)
            .unwrap_or(0),
        validation_retry_limit: recovery
            .map(|state| state.max_validation_retries)
            .unwrap_or(0),
        next_validation_retry_at: recovery.and_then(|state| state.next_validation_retry_at.clone()),
        heartbeat_status: heartbeat_status(project),
        automated_test_status,
        code_review_status,
        review_protocol_status,
        acceptance_evidence_status: acceptance_evidence_status(ledger, test),
        control_lock_valid: None,
        control_action_description: String::new(),
        control_action_elapsed_seconds: 0,
        control_lock_last_heartbeat_at: None,
        control_lock_failure_reason: String::new(),
        control_lock_cleanup_available: false,
    }
}

fn recovery_task(project: &Project) -> Option<&crate::project::Subtask> {
    let task_id = project
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|state| state.subtask_id.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            project
                .execution_session
                .as_ref()
                .map(|session| session.subtask_id.as_str())
                .filter(|value| !value.is_empty())
        })?;
    crate::task_tree::find_task(project, task_id).ok().flatten()
}

fn affected_task_label(project: &Project, task: Option<&crate::project::Subtask>) -> String {
    task.map(|value| value.title.clone())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            project.execution_session.as_ref().and_then(|session| {
                (!session.subtask_title.is_empty())
                    .then(|| session.subtask_title.clone())
                    .or_else(|| {
                        (!session.subtask_id.is_empty()).then(|| session.subtask_id.clone())
                    })
            })
        })
        .unwrap_or_default()
}

fn baseline_reference(project: &Project) -> String {
    let value = project
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|state| state.baseline_commit.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            project
                .execution_session
                .as_ref()
                .map(|session| session.base_commit.as_str())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    value.chars().take(12).collect()
}

fn baseline_impact_summary(project: &Project) -> String {
    let Some(recovery) = project.workflow_state.recovery_state.as_ref() else {
        return "执行基线恢复结果未记录。".to_string();
    };
    match recovery.baseline_status {
        crate::project::RecoveryBaselineStatus::Unknown => "执行基线恢复结果未记录。".to_string(),
        crate::project::RecoveryBaselineStatus::NotRequired => {
            "本轮恢复不需要恢复执行基线。".to_string()
        }
        crate::project::RecoveryBaselineStatus::Pending => "执行基线恢复尚未完成。".to_string(),
        crate::project::RecoveryBaselineStatus::Restored if recovery.baseline_stash_created => {
            "未提交改动已暂存，工作区已恢复到执行基线。".to_string()
        }
        crate::project::RecoveryBaselineStatus::Restored => "工作区已恢复到执行基线。".to_string(),
        crate::project::RecoveryBaselineStatus::RestoreFailed => {
            "执行基线恢复失败，需要人工检查工作区。".to_string()
        }
    }
}

fn validation_phase_label(project: &Project, test: Option<&crate::project::TestResult>) -> String {
    use crate::project::VerificationStage;
    let session_stage = project
        .execution_session
        .as_ref()
        .map(|session| &session.verification_stage);
    let stage = session_stage
        .filter(|stage| **stage != VerificationStage::NotStarted)
        .cloned()
        .or_else(|| test.map(|value| value.verification_stage.clone()));
    match stage {
        Some(VerificationStage::NotStarted) | None => "等待验证",
        Some(VerificationStage::AutomatedTests) => "运行自动化测试",
        Some(VerificationStage::PreparingEvidence) => "准备验收证据",
        Some(VerificationStage::RequestingReview) => "请求代码审查",
        Some(VerificationStage::ParsingReview) => "解析审查结果",
        Some(VerificationStage::DeterministicNormalization) => "确定性归一化",
        Some(VerificationStage::ProtocolRepair) => "修复审查协议",
        Some(VerificationStage::ReviewRetry) => "重新请求代码审查",
        Some(VerificationStage::TargetedEvidence) => "定向补充证据",
        Some(VerificationStage::Completed) => "验证完成",
    }
    .to_string()
}

fn heartbeat_status_at(project: &Project, now: chrono::DateTime<chrono::Utc>) -> String {
    let Some(autopilot) = project.workflow_state.autopilot_state.as_ref() else {
        return "未启动".to_string();
    };
    let waiting_human = project
        .workflow_state
        .recovery_state
        .as_ref()
        .map(|state| state.phase == RecoveryPhase::WaitingHuman)
        .unwrap_or(false)
        || autopilot.run_status == crate::project::AutopilotRunStatus::ErrorStopped
        || autopilot.recovery_action == AutopilotRecoveryAction::WaitHumanDecision;
    if waiting_human {
        return if autopilot.heartbeat_at.is_empty() {
            "已停止".to_string()
        } else {
            format!("已停止，最后更新 {}", autopilot.heartbeat_at)
        };
    }
    if autopilot.heartbeat_at.is_empty() {
        return "未记录".to_string();
    }
    let stale = chrono::DateTime::parse_from_rfc3339(&autopilot.heartbeat_at)
        .ok()
        .is_some_and(|heartbeat| {
            autopilot.active
                && autopilot.run_status == crate::project::AutopilotRunStatus::Running
                && now.signed_duration_since(heartbeat.with_timezone(&chrono::Utc))
                    > chrono::Duration::seconds(15)
        });
    if stale {
        format!("异常，最后更新 {}", autopilot.heartbeat_at)
    } else {
        format!("正常，最后更新 {}", autopilot.heartbeat_at)
    }
}

fn heartbeat_status(project: &Project) -> String {
    heartbeat_status_at(project, chrono::Utc::now())
}

fn quality_statuses(test: Option<&crate::project::TestResult>) -> (String, String, String) {
    use crate::project::{
        AutomatedTestStatus, ReviewEvidenceStatus, ReviewFailureKind, ReviewIssueSeverity,
        ReviewStatus, VerificationKind, VerificationStage,
    };
    let automated = match test.map(|value| &value.automated_test_status) {
        Some(AutomatedTestStatus::Passed) => "自动化测试：通过",
        Some(AutomatedTestStatus::Failed) => "自动化测试：失败",
        Some(AutomatedTestStatus::NotConfigured) => "自动化测试：未配置",
        Some(AutomatedTestStatus::Unavailable) => "自动化测试：不可用",
        Some(AutomatedTestStatus::Unknown) | None => "自动化测试：状态未知",
    };
    let protocol_failure = test.is_some_and(|value| {
        matches!(
            value.review_failure_kind.as_ref(),
            Some(ReviewFailureKind::InvalidJson | ReviewFailureKind::FieldTypeMismatch)
        )
    });
    let service_failure =
        test.is_some_and(|value| value.review_failure_kind.is_some() && !protocol_failure);
    let blocking_review = test.is_some_and(|value| {
        value
            .review_issues
            .iter()
            .any(|issue| issue.severity.as_ref() == Some(&ReviewIssueSeverity::Blocking))
    });
    let code_review = match test {
        Some(value) if value.verification_kind == VerificationKind::DeterministicLocal => {
            if value.passed {
                "本地确定性验证：通过"
            } else {
                "本地确定性验证：未通过"
            }
        }
        Some(value)
            if service_failure
                || value.review_evidence_status == ReviewEvidenceStatus::Unavailable =>
        {
            "代码审查：不可用"
        }
        Some(value) if value.review_passed => "代码审查：通过",
        Some(_) if blocking_review => "代码审查：存在阻断问题",
        _ => "代码审查：待确认",
    };
    let protocol_in_progress = test.is_some_and(|value| {
        value.review_status == ReviewStatus::InProgress
            || matches!(
                &value.verification_stage,
                VerificationStage::ParsingReview
                    | VerificationStage::DeterministicNormalization
                    | VerificationStage::ProtocolRepair
                    | VerificationStage::ReviewRetry
            )
    });
    let protocol = match test {
        Some(_) if protocol_failure => "审查协议：格式异常",
        Some(_) if protocol_in_progress => "审查协议：处理中",
        Some(value) if value.review_status == ReviewStatus::Completed => "审查协议：有效",
        Some(value) if value.review_status == ReviewStatus::Failed => "审查协议：未取得结果",
        _ => "审查协议：未请求",
    };
    (
        automated.to_string(),
        code_review.to_string(),
        protocol.to_string(),
    )
}

fn acceptance_evidence_status(
    ledger: &[crate::project::AcceptanceLedgerItem],
    test: Option<&crate::project::TestResult>,
) -> String {
    use crate::project::{AcceptanceStatus, ReviewEvidenceStatus};
    if ledger
        .iter()
        .any(|item| item.status == AcceptanceStatus::Contradictory)
    {
        "验收证据：结论冲突"
    } else if ledger
        .iter()
        .any(|item| item.status == AcceptanceStatus::Unknown)
    {
        "验收证据：不足"
    } else if ledger
        .iter()
        .any(|item| item.status == AcceptanceStatus::Unsatisfied)
    {
        "验收证据：存在阻断项"
    } else if !ledger.is_empty() {
        "验收证据：充分"
    } else if test
        .is_some_and(|value| value.review_evidence_status == ReviewEvidenceStatus::Unavailable)
    {
        "验收证据：不可用"
    } else {
        "验收证据：无逐项标准"
    }
    .to_string()
}

fn recovery_phase_label(project: &Project, kind: &RecoveryPresentationKind) -> String {
    if let Some(recovery) = project.workflow_state.recovery_state.as_ref() {
        return match recovery.phase {
            RecoveryPhase::Diagnosing => "正在诊断".to_string(),
            RecoveryPhase::Repairing => "正在修复".to_string(),
            RecoveryPhase::Retesting => "正在复测".to_string(),
            RecoveryPhase::Replanning => "正在重新规划".to_string(),
            RecoveryPhase::WaitingHuman => "等待人工决策".to_string(),
            RecoveryPhase::WaitingEngine => "等待执行引擎恢复".to_string(),
            RecoveryPhase::Recovered => "恢复已完成".to_string(),
        };
    }
    match kind {
        RecoveryPresentationKind::ControlActionOccupied => "等待控制动作收口".to_string(),
        RecoveryPresentationKind::GitReconfirmation => "等待 Git 确认".to_string(),
        RecoveryPresentationKind::BaselineRecovery => "等待基线恢复".to_string(),
        RecoveryPresentationKind::SyncAndClose => "等待最终状态同步".to_string(),
        RecoveryPresentationKind::None => String::new(),
        _ => "等待恢复动作".to_string(),
    }
}

fn post_action_expectation(
    kind: &RecoveryPresentationKind,
    primary_action: Option<&RecoveryActionPresentation>,
) -> String {
    if let Some(action) = primary_action {
        return match action.capability {
            RecoveryCapability::ClearStaleControlLock => {
                "后端将释放陈旧锁，并依据执行会话或 Git 确认事务事实恢复任务状态。".to_string()
            }
            RecoveryCapability::RetryGitConfirmation => {
                "续跑原 Git 确认事务，不重新执行代码任务或质量验证。".to_string()
            }
            RecoveryCapability::AcknowledgeExecutionRecovery => {
                "确认影响后恢复执行基线，再由后端决定是否继续后台作业。".to_string()
            }
            RecoveryCapability::ResolveHumanRecovery => {
                "应用所选人工决策，并返回最新运行时状态。".to_string()
            }
            RecoveryCapability::SyncProject => "只读取后端最终状态，不改动代码。".to_string(),
            _ => "动作完成后将返回统一运行时快照。".to_string(),
        };
    }
    match kind {
        RecoveryPresentationKind::ControlActionOccupied => {
            "控制动作结束或陈旧锁清理后将立即刷新项目状态。".to_string()
        }
        RecoveryPresentationKind::AutomaticRecovery
        | RecoveryPresentationKind::ValidationRetry
        | RecoveryPresentationKind::EngineBlocked => {
            "后台重试结束后将自动同步最终状态。".to_string()
        }
        RecoveryPresentationKind::SyncAndClose => {
            "代码动作已经完成，只需读取后端最终状态。".to_string()
        }
        _ => String::new(),
    }
}

fn control_action_label(kind: &str) -> &str {
    match kind {
        "split" => "拆分任务",
        "execute" => "执行任务",
        "local_validate" => "本地验证",
        "automated_validate" => "自动验证",
        "targeted_validate" => "定向审查",
        "repair" => "修复任务",
        "recompile" => "重编译任务",
        "accept_deviation" => "接受偏差",
        "git_confirm" => "Git 确认",
        "wait" => "等待",
        "human" => "人工处理",
        _ => "控制动作",
    }
}

fn present_control_action_occupancy(
    project: &Project,
    occupancy: crate::control_action_executor::ControlActionOccupancy,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<RecoveryPresentation> {
    use crate::control_action_executor::ControlActionOccupancy;

    let (lease, valid, cleanup_available, failure_reason, owner_description) = match occupancy {
        ControlActionOccupancy::Unoccupied => return None,
        ControlActionOccupancy::ActiveLocal(lease) => {
            (Some(lease), true, false, String::new(), "当前进程")
        }
        ControlActionOccupancy::ActiveForeign(lease) => {
            (Some(lease), true, false, String::new(), "另一 Metheus 进程")
        }
        ControlActionOccupancy::Stale { lease, reason } => {
            let cleanup_available =
                crate::control_action_executor::stale_control_action_can_be_cleared(
                    lease.as_ref(),
                    crate::project_state_bus::process_start_id(),
                    now,
                );
            (
                lease,
                !cleanup_available,
                cleanup_available,
                reason,
                "原持有进程",
            )
        }
    };
    let action_id = lease
        .as_ref()
        .map(|value| value.action_id.as_str())
        .unwrap_or(project.task_control.active_action_id.as_str());
    let action_kind = lease
        .as_ref()
        .map(|value| value.action_kind.as_str())
        .unwrap_or(project.task_control.active_action_kind.as_str());
    let task_id = lease
        .as_ref()
        .map(|value| value.task_id.as_str())
        .unwrap_or(project.task_control.active_action_task_id.as_str());
    let elapsed = lease
        .as_ref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value.started_at).ok())
        .map(|started| {
            now.signed_duration_since(started.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0) as u64
        })
        .unwrap_or(0);
    let heartbeat_at = lease.as_ref().map(|value| value.heartbeat_at.clone());
    let action_description = format!(
        "{} · 动作 {}{}",
        control_action_label(action_kind),
        if action_id.is_empty() {
            "未知"
        } else {
            action_id
        },
        if task_id.is_empty() {
            String::new()
        } else {
            format!(" · 任务 {}", task_id)
        }
    );
    let reason_text = if valid {
        format!(
            "{}正在执行{}，已持续 {} 秒；后端心跳仍有效。",
            owner_description,
            control_action_label(action_kind),
            elapsed
        )
    } else {
        format!("控制动作锁已失效：{}", failure_reason)
    };
    let primary = if cleanup_available {
        action(
            RecoveryCapability::ClearStaleControlLock,
            "清理陈旧锁并恢复操作",
        )
    } else {
        action(RecoveryCapability::SyncProject, "等待当前动作完成")
    };
    let mut presentation = finish(
        project,
        RecoveryPresentationKind::ControlActionOccupied,
        if valid {
            "控制动作正在执行"
        } else {
            "控制动作锁已失效"
        },
        reason_text,
        if valid {
            RecoverySeverity::Info
        } else {
            RecoverySeverity::Warning
        },
        Some(primary),
        Vec::new(),
        true,
        false,
        false,
        false,
        Vec::new(),
    );
    presentation.control_lock_valid = Some(valid);
    presentation.control_action_description = action_description;
    presentation.control_action_elapsed_seconds = elapsed;
    presentation.control_lock_last_heartbeat_at = heartbeat_at.clone();
    presentation.control_lock_failure_reason = failure_reason;
    presentation.control_lock_cleanup_available = cleanup_available;
    presentation.phase_label = if valid {
        "等待后台动作完成".to_string()
    } else {
        "等待后端清理陈旧锁".to_string()
    };
    presentation.heartbeat_status = heartbeat_at
        .map(|value| format!("最后更新 {}", value))
        .unwrap_or_else(|| "缺少可验证心跳".to_string());
    presentation.affected_task_label = crate::task_tree::find_task(project, task_id)
        .ok()
        .flatten()
        .map(|task| {
            if task.title.is_empty() {
                task.id.clone()
            } else {
                task.title.clone()
            }
        })
        .unwrap_or_else(|| task_id.to_string());
    presentation.post_action_expectation = if valid {
        "动作正常完成并由后端释放租约后，任务操作会自动恢复。".to_string()
    } else {
        "后端清理锁后会按磁盘执行事实重新开放任务决策；Git 确认只续跑原事务。".to_string()
    };
    presentation.stale_risk = !valid;
    presentation.sync_needed = true;
    presentation.sync_risk_summary = if valid {
        "有效占用不会被强制清理。".to_string()
    } else {
        "锁清理由后端裁决，前端不会自行判断或改写项目文件。".to_string()
    };
    Some(presentation)
}

fn human_decision_options(project: &Project) -> Vec<RecoveryDecisionOption> {
    let recovery = project.workflow_state.recovery_state.as_ref();
    if recovery.is_some_and(|state| validation_recovery(&state.error_kind)) {
        return vec![decision(
            RecoveryDecisionResolution::Revalidate,
            "重新验证",
            false,
            false,
            false,
        )];
    }

    let mut options = vec![decision(
        RecoveryDecisionResolution::Retest,
        if recovery.is_some_and(|state| state.error_kind == RecoveryErrorKind::EvidenceInsufficient)
        {
            "补充证据后复测"
        } else {
            "人工修复后复测"
        },
        false,
        false,
        false,
    )];
    options.push(decision(
        RecoveryDecisionResolution::RestoreAndRetry,
        "恢复基线并重试",
        false,
        false,
        true,
    ));

    if recovery.is_some_and(|state| state.replan_attempted) {
        options.push(disabled_decision(
            RecoveryDecisionResolution::RegeneratePlan,
            "重新规划当前任务",
            "当前任务已经执行过一次受限重规划",
            false,
            false,
            false,
        ));
    } else {
        options.push(decision(
            RecoveryDecisionResolution::RegeneratePlan,
            "重新规划当前任务",
            false,
            false,
            false,
        ));
    }

    let subtask = recovery
        .map(|state| state.subtask_id.as_str())
        .filter(|id| !id.is_empty())
        .or_else(|| {
            project
                .execution_session
                .as_ref()
                .map(|session| session.subtask_id.as_str())
        })
        .and_then(|id| crate::task_tree::find_task(project, id).ok().flatten());
    let task_id = subtask.map(|task| task.id.as_str()).unwrap_or_default();
    let acceptance_option = |action, resolution, label, select| {
        let policy = crate::human_action_policy::evaluate(project, task_id, action);
        if policy.allowed {
            decision(resolution, label, true, select, false)
        } else {
            disabled_decision(
                resolution,
                label,
                &policy.denial_reason,
                true,
                select,
                false,
            )
        }
    };
    options.push(acceptance_option(
        crate::human_action_policy::HumanTerminalAction::ConfirmActualPass,
        RecoveryDecisionResolution::ConfirmActualPass,
        "确认实际通过",
        false,
    ));
    options.push(acceptance_option(
        crate::human_action_policy::HumanTerminalAction::AcceptDeviation,
        RecoveryDecisionResolution::AcceptDeviation,
        "接受偏差并继续",
        true,
    ));
    let skip = crate::human_action_policy::evaluate(
        project,
        task_id,
        crate::human_action_policy::HumanTerminalAction::SkipTask,
    );
    options.push(if skip.allowed {
        decision(
            RecoveryDecisionResolution::SkipTask,
            "跳过当前任务",
            true,
            false,
            skip.requires_preview,
        )
    } else {
        disabled_decision(
            RecoveryDecisionResolution::SkipTask,
            "跳过当前任务",
            &skip.denial_reason,
            true,
            false,
            skip.requires_preview,
        )
    });
    options
}

pub(crate) fn present_recovery(project: &Project) -> RecoveryPresentation {
    let now = chrono::Utc::now();
    let occupancy = crate::control_action_executor::classify_control_action_occupancy(
        &project.task_control,
        crate::project_state_bus::process_start_id(),
        now,
    );
    if let Some(presentation) = present_control_action_occupancy(project, occupancy, now) {
        return presentation;
    }
    let session = project.execution_session.as_ref();
    let session_status = session
        .map(|value| value.status.to_ascii_lowercase())
        .unwrap_or_default();
    let recovery = project.workflow_state.recovery_state.as_ref();
    let autopilot = project.workflow_state.autopilot_state.as_ref();
    let recovery_action = autopilot
        .map(|state| &state.recovery_action)
        .unwrap_or(&AutopilotRecoveryAction::None);
    let automatic_infrastructure_retry = autopilot
        .map(|state| state.transient_retry_count > 0 && state.next_retry_at.is_some())
        .unwrap_or(false);

    if session_status == "confirmation_blocked" {
        let retryable = retryable_git_confirmation(
            session.and_then(|value| value.confirmation_failure_kind.as_ref()),
        );
        let primary = if retryable {
            action(RecoveryCapability::RetryGitConfirmation, "重新确认提交")
        } else {
            disabled_action(
                RecoveryCapability::RetryGitConfirmation,
                "等待人工核对 Git",
                "当前 Git 冲突不允许系统覆盖、移动或删除标签",
            )
        };
        return finish(
            project,
            RecoveryPresentationKind::GitReconfirmation,
            "Git 确认受阻",
            git_reconfirmation_reason(project),
            RecoverySeverity::Error,
            Some(primary),
            Vec::new(),
            true,
            false,
            false,
            false,
            Vec::new(),
        );
    }

    if recovery
        .map(|state| {
            state.error_kind == RecoveryErrorKind::EngineBlocked
                || state.phase == RecoveryPhase::WaitingEngine
        })
        .unwrap_or(false)
    {
        let supports_preview = !automatic_infrastructure_retry;
        return finish(
            project,
            RecoveryPresentationKind::EngineBlocked,
            "执行引擎阻断",
            reason(project, "请修复引擎认证、额度或服务状态后重试。"),
            RecoverySeverity::Error,
            (!automatic_infrastructure_retry).then(|| {
                action(
                    RecoveryCapability::AcknowledgeExecutionRecovery,
                    "检查引擎并重试",
                )
            }),
            Vec::new(),
            false,
            true,
            supports_preview,
            automatic_infrastructure_retry,
            Vec::new(),
        );
    }

    if let Some(recovery) = recovery {
        if validation_recovery(&recovery.error_kind) {
            let automatic_retry = recovery.phase == RecoveryPhase::Retesting
                && recovery.validation_retry_count < recovery.max_validation_retries;
            return finish(
                project,
                RecoveryPresentationKind::ValidationRetry,
                if automatic_retry {
                    "等待验证重试"
                } else {
                    "验证需要重新执行"
                },
                reason(project, "验证服务或返回协议暂时不可用，代码不会回退。"),
                RecoverySeverity::Warning,
                (!automatic_retry)
                    .then(|| action(RecoveryCapability::ResolveHumanRecovery, "重新验证")),
                Vec::new(),
                true,
                false,
                false,
                automatic_retry,
                (!automatic_retry)
                    .then(|| {
                        decision(
                            RecoveryDecisionResolution::Revalidate,
                            "重新验证",
                            false,
                            false,
                            false,
                        )
                    })
                    .into_iter()
                    .collect(),
            );
        }

        if recovery.error_kind == RecoveryErrorKind::EvidenceInsufficient {
            return finish(
                project,
                RecoveryPresentationKind::EvidenceInsufficient,
                "验收证据不足",
                reason(project, "需要补充证据后重新验证，当前代码不会回退。"),
                RecoverySeverity::Warning,
                Some(action(
                    RecoveryCapability::ResolveHumanRecovery,
                    "补充证据并重新验证",
                )),
                Vec::new(),
                true,
                false,
                false,
                false,
                vec![decision(
                    RecoveryDecisionResolution::Retest,
                    "补充证据后复测",
                    false,
                    false,
                    false,
                )],
            );
        }

        let human_kind = matches!(
            recovery.error_kind,
            RecoveryErrorKind::HumanRequired
                | RecoveryErrorKind::ContractContradiction
                | RecoveryErrorKind::ValidationOscillation
                | RecoveryErrorKind::AutomatedTestUnavailable
                | RecoveryErrorKind::TestUnavailable
                | RecoveryErrorKind::StateConflict
                | RecoveryErrorKind::ValidationFailure
        );
        if recovery.phase == RecoveryPhase::WaitingHuman || human_kind {
            return finish(
                project,
                RecoveryPresentationKind::HumanDecision,
                "等待人工决策",
                reason(project, "系统无法安全自动决定下一步，请选择人工处理方式。"),
                RecoverySeverity::Error,
                Some(action(
                    RecoveryCapability::ResolveHumanRecovery,
                    "选择处理方式",
                )),
                Vec::new(),
                true,
                false,
                false,
                false,
                human_decision_options(project),
            );
        }
    }

    let recoverable_session = session
        .map(|value| value.is_recoverable_failure())
        .unwrap_or(false);
    if *recovery_action == AutopilotRecoveryAction::RestoreExecutionBaseline
        || (recovery.is_none() && recoverable_session)
    {
        return finish(
            project,
            RecoveryPresentationKind::BaselineRecovery,
            match session_status.as_str() {
                "session_lost" => "执行中断",
                "stop_failed" => "暂停失败",
                _ => "执行失败",
            },
            reason(project, "需要先预览并恢复本次执行基线。"),
            RecoverySeverity::Error,
            Some(action(
                RecoveryCapability::AcknowledgeExecutionRecovery,
                "预览并恢复执行基线",
            )),
            Vec::new(),
            false,
            true,
            true,
            false,
            Vec::new(),
        );
    }

    if recovery.is_some() || *recovery_action == AutopilotRecoveryAction::RunAutomaticRecovery {
        let automatic = recovery
            .map(|state| state.phase != RecoveryPhase::WaitingHuman)
            .unwrap_or(false);
        return finish(
            project,
            RecoveryPresentationKind::AutomaticRecovery,
            if automatic {
                "自动恢复中"
            } else {
                "质量恢复受阻"
            },
            reason(project, "系统正在执行受限诊断、修复和复测。"),
            if automatic {
                RecoverySeverity::Warning
            } else {
                RecoverySeverity::Error
            },
            (!automatic).then(|| action(RecoveryCapability::RunAutomaticRecovery, "继续自动恢复")),
            Vec::new(),
            true,
            false,
            false,
            automatic,
            Vec::new(),
        );
    }

    let (kind, title, capability, label) = match recovery_action {
        AutopilotRecoveryAction::RetryAutopilotAdvance => (
            RecoveryPresentationKind::RetryAdvance,
            "自动推进受阻",
            RecoveryCapability::RetryAutopilotAdvance,
            "重试自动推进",
        ),
        AutopilotRecoveryAction::RegenerateExecutionPlan => (
            RecoveryPresentationKind::RegeneratePlan,
            "执行计划需要重生成",
            RecoveryCapability::RegenerateExecutionPlan,
            "重生成执行计划",
        ),
        AutopilotRecoveryAction::PrepareExecutionWorkspace => (
            RecoveryPresentationKind::PrepareWorkspace,
            "Git 工作区尚未就绪",
            RecoveryCapability::PrepareExecutionWorkspace,
            "准备执行工作区",
        ),
        AutopilotRecoveryAction::ResolveWorkspaceChanges => (
            RecoveryPresentationKind::ResolveWorkspaceChanges,
            "工作区存在外部变更",
            RecoveryCapability::RefreshExecutionWorkspace,
            "重新检查工作区",
        ),
        AutopilotRecoveryAction::SyncAndClose => (
            RecoveryPresentationKind::SyncAndClose,
            "运行状态需要收口",
            RecoveryCapability::CloseAutopilot,
            "同步并关闭",
        ),
        AutopilotRecoveryAction::RetryGitConfirmation => (
            RecoveryPresentationKind::GitReconfirmation,
            "Git 确认受阻",
            RecoveryCapability::RetryGitConfirmation,
            "重新确认提交",
        ),
        AutopilotRecoveryAction::WaitHumanDecision if recovery.is_some() => (
            RecoveryPresentationKind::HumanDecision,
            "等待人工决策",
            RecoveryCapability::ResolveHumanRecovery,
            "选择处理方式",
        ),
        AutopilotRecoveryAction::WaitHumanDecision => return RecoveryPresentation::none(project),
        AutopilotRecoveryAction::None
        | AutopilotRecoveryAction::RestoreExecutionBaseline
        | AutopilotRecoveryAction::RunAutomaticRecovery => {
            return RecoveryPresentation::none(project);
        }
    };

    let decision_options = if kind == RecoveryPresentationKind::HumanDecision {
        human_decision_options(project)
    } else {
        Vec::new()
    };
    finish(
        project,
        kind,
        title,
        reason(project, "后台运行已停止，请执行建议的恢复动作。"),
        RecoverySeverity::Error,
        Some(action(capability, label)),
        Vec::new(),
        true,
        false,
        false,
        false,
        decision_options,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AutopilotState, ExecutionSession, RecoveryState};

    fn autopilot_project(action: AutopilotRecoveryAction) -> Project {
        let mut project = Project::new("recovery-presentation");
        project.workflow_state.autopilot_state = Some(AutopilotState {
            recovery_action: action,
            ..AutopilotState::default()
        });
        project
    }

    #[test]
    fn phase1_runtime_contract_failed_execution_requires_only_baseline_recovery() {
        let mut project = Project::new("baseline-recovery");
        project.execution_session = Some(ExecutionSession {
            execution_id: "execution-1".to_string(),
            status: "execution_failed".to_string(),
            ..ExecutionSession::default()
        });

        let presentation = present_recovery(&project);
        assert_eq!(
            presentation.kind,
            RecoveryPresentationKind::BaselineRecovery
        );
        assert!(presentation.requires_baseline_restore);
        assert!(presentation.supports_preview);
        assert_eq!(presentation.code_impact_summary, "执行基线恢复结果未记录。");
    }

    #[test]
    fn git_confirmation_never_offers_baseline_restore() {
        let mut project = Project::new("git-reconfirmation");
        project.execution_session = Some(ExecutionSession {
            execution_id: "execution-2".to_string(),
            status: "confirmation_blocked".to_string(),
            confirmation_failure_kind: Some(GitConfirmationFailureKind::CommitFailed),
            ..ExecutionSession::default()
        });

        let presentation = present_recovery(&project);
        assert_eq!(
            presentation.kind,
            RecoveryPresentationKind::GitReconfirmation
        );
        assert!(presentation.preserve_current_code);
        assert!(!presentation.requires_baseline_restore);
        assert!(presentation
            .capabilities
            .contains(&RecoveryCapability::RetryGitConfirmation));
    }

    #[test]
    fn runtime_fault_block_message_adds_preservation_notice_exactly_once() {
        let mut project = Project::new("git-message-dedup");
        project.execution_session = Some(ExecutionSession {
            status: "confirmation_blocked".to_string(),
            failure_message: "Git 标签身份冲突，请人工核对。".to_string(),
            confirmation_failure_kind: Some(GitConfirmationFailureKind::V2TagIntegrityConflict),
            ..ExecutionSession::default()
        });

        let presentation = present_recovery(&project);
        assert_eq!(
            presentation.reason.matches("代码与质量结果已保留").count(),
            1
        );
        assert!(!presentation
            .post_action_expectation
            .contains("代码与质量结果已保留"));

        project.execution_session.as_mut().unwrap().failure_message =
            "历史原因；代码与质量结果已保留。".to_string();
        let historical = present_recovery(&project);
        assert_eq!(historical.reason.matches("代码与质量结果已保留").count(), 1);
    }

    #[test]
    fn phase1_runtime_contract_validation_and_evidence_failures_preserve_code() {
        let mut validation = Project::new("validation-retry");
        validation.workflow_state.recovery_state = Some(RecoveryState {
            error_kind: RecoveryErrorKind::ReviewProtocolFailure,
            phase: RecoveryPhase::Retesting,
            validation_retry_count: 1,
            max_validation_retries: 3,
            ..RecoveryState::default()
        });
        let validation_view = present_recovery(&validation);
        assert_eq!(
            validation_view.kind,
            RecoveryPresentationKind::ValidationRetry
        );
        assert!(validation_view.automatic_retry);
        assert!(!validation_view.requires_baseline_restore);
        assert!(!validation_view.background_retry_active);
        assert!(validation_view.background_retry_summary.is_empty());

        let mut evidence = Project::new("evidence-recovery");
        evidence.workflow_state.recovery_state = Some(RecoveryState {
            error_kind: RecoveryErrorKind::EvidenceInsufficient,
            phase: RecoveryPhase::WaitingHuman,
            ..RecoveryState::default()
        });
        let evidence_view = present_recovery(&evidence);
        assert_eq!(
            evidence_view.kind,
            RecoveryPresentationKind::EvidenceInsufficient
        );
        assert!(evidence_view.preserve_current_code);
        assert!(!evidence_view.requires_baseline_restore);
    }

    #[test]
    fn engine_and_human_blocks_are_distinct() {
        let mut engine = Project::new("engine-blocked");
        engine.workflow_state.recovery_state = Some(RecoveryState {
            error_kind: RecoveryErrorKind::EngineBlocked,
            phase: RecoveryPhase::WaitingEngine,
            ..RecoveryState::default()
        });
        assert_eq!(
            present_recovery(&engine).kind,
            RecoveryPresentationKind::EngineBlocked
        );

        let mut human = autopilot_project(AutopilotRecoveryAction::WaitHumanDecision);
        human.name = "human-decision".to_string();
        human.workflow_state.recovery_state = Some(RecoveryState {
            error_kind: RecoveryErrorKind::HumanRequired,
            phase: RecoveryPhase::WaitingHuman,
            ..RecoveryState::default()
        });
        assert_eq!(
            present_recovery(&human).kind,
            RecoveryPresentationKind::HumanDecision
        );
    }

    fn project_with_control_lease(
        owner: &str,
        started_at: String,
        heartbeat_at: String,
    ) -> Project {
        let mut project = Project::new("control-lock-presentation");
        let lease = crate::task_control::ControlActionLease {
            action_id: "control-action-1".to_string(),
            owner_process_start_id: owner.to_string(),
            action_kind: "execute".to_string(),
            task_id: "task-1".to_string(),
            started_at,
            heartbeat_at,
            expected_max_duration_secs: 1_200,
        };
        project.task_control.active_action_id = lease.action_id.clone();
        project.task_control.active_action_kind = lease.action_kind.clone();
        project.task_control.active_action_task_id = lease.task_id.clone();
        project.task_control.active_action_lease = Some(lease);
        project
    }

    #[test]
    fn runtime_fault_control_lock_presentation_active_only_offers_wait() {
        let now = chrono::Utc::now().to_rfc3339();
        let project = project_with_control_lease(
            crate::project_state_bus::process_start_id(),
            now.clone(),
            now,
        );
        let presentation = present_recovery(&project);

        assert_eq!(
            presentation.kind,
            RecoveryPresentationKind::ControlActionOccupied
        );
        assert_eq!(presentation.control_lock_valid, Some(true));
        assert!(!presentation.control_lock_cleanup_available);
        assert_eq!(
            presentation
                .primary_action
                .as_ref()
                .map(|action| &action.capability),
            Some(&RecoveryCapability::SyncProject)
        );
        assert!(presentation.secondary_actions.is_empty());
        assert!(presentation.decision_options.is_empty());
        assert!(!presentation
            .capabilities
            .contains(&RecoveryCapability::ResolveHumanRecovery));
    }

    #[test]
    fn runtime_fault_control_lock_presentation_stale_only_offers_backend_cleanup() {
        let now = chrono::Utc::now();
        let project = project_with_control_lease(
            "old-process",
            (now - chrono::Duration::seconds(40)).to_rfc3339(),
            (now - chrono::Duration::seconds(20)).to_rfc3339(),
        );
        let presentation = present_recovery(&project);

        assert_eq!(
            presentation.kind,
            RecoveryPresentationKind::ControlActionOccupied
        );
        assert_eq!(presentation.control_lock_valid, Some(false));
        assert!(presentation.control_lock_cleanup_available);
        assert_eq!(
            presentation
                .primary_action
                .as_ref()
                .map(|action| &action.capability),
            Some(&RecoveryCapability::ClearStaleControlLock)
        );
        assert!(presentation.secondary_actions.is_empty());
        assert!(presentation.decision_options.is_empty());
    }

    #[test]
    fn runtime_fault_control_lock_presentation_precedes_code_recovery() {
        let now = chrono::Utc::now().to_rfc3339();
        let mut project = project_with_control_lease(
            crate::project_state_bus::process_start_id(),
            now.clone(),
            now,
        );
        project.workflow_state.recovery_state = Some(RecoveryState {
            phase: RecoveryPhase::WaitingHuman,
            error_kind: RecoveryErrorKind::HumanRequired,
            ..RecoveryState::default()
        });

        assert_eq!(
            present_recovery(&project).kind,
            RecoveryPresentationKind::ControlActionOccupied
        );
    }

    #[test]
    fn runtime_fault_control_lock_presentation_never_invents_human_recovery_without_state() {
        let project = autopilot_project(AutopilotRecoveryAction::WaitHumanDecision);
        let presentation = present_recovery(&project);
        assert_eq!(presentation.kind, RecoveryPresentationKind::None);
        assert!(presentation.primary_action.is_none());
        assert!(presentation.decision_options.is_empty());
    }

    #[test]
    fn recovery_progress_truth_table_is_truthful() {
        let now = chrono::Utc::now();
        let recovery_state = |updated_at: String| RecoveryState {
            phase: RecoveryPhase::Replanning,
            error_kind: RecoveryErrorKind::PlanFailure,
            subtask_id: "sub-1".to_string(),
            updated_at,
            ..RecoveryState::default()
        };
        let autopilot_state = || AutopilotState {
            active: true,
            run_status: crate::project::AutopilotRunStatus::Running,
            recovery_action: AutopilotRecoveryAction::RunAutomaticRecovery,
            ..AutopilotState::default()
        };

        let mut queued = Project::new("progress-queued");
        queued.workflow_state.recovery_state = Some(recovery_state(
            (now - chrono::Duration::seconds(10)).to_rfc3339(),
        ));
        queued.workflow_state.autopilot_state = Some(autopilot_state());
        assert_eq!(
            recovery_progress_at(&queued, now, true).status,
            RecoveryProgressStatus::Queued
        );
        assert!(!present_recovery(&queued).background_retry_active);

        let mut scheduled = queued.clone();
        scheduled.name = "progress-scheduled".to_string();
        scheduled
            .workflow_state
            .autopilot_state
            .as_mut()
            .expect("autopilot")
            .next_retry_at = Some((now + chrono::Duration::seconds(60)).to_rfc3339());
        assert_eq!(
            recovery_progress_at(&scheduled, now, true).status,
            RecoveryProgressStatus::Scheduled
        );
        assert!(present_recovery(&scheduled).background_retry_active);

        let claimed_project = |name: &str, progress_age: i64, action_age: i64| {
            let mut project = Project::new(name);
            project.workflow_state.recovery_state = Some(recovery_state(
                (now - chrono::Duration::seconds(progress_age)).to_rfc3339(),
            ));
            project.workflow_state.autopilot_state = Some(AutopilotState {
                current_action_id: format!("{name}-claim"),
                current_action_kind: "run_error_recovery".to_string(),
                action_started_at: (now - chrono::Duration::seconds(action_age)).to_rfc3339(),
                heartbeat_at: now.to_rfc3339(),
                ..autopilot_state()
            });
            project
        };

        let running = claimed_project("progress-running", 10, 30);
        let running_progress = recovery_progress_at(&running, now, true);
        assert_eq!(running_progress.status, RecoveryProgressStatus::Running);
        assert_eq!(running_progress.elapsed_seconds, Some(30));
        assert!(running_progress.warning_at.is_some());
        assert!(running_progress.hard_deadline_at.is_some());
        assert!(present_recovery(&running).background_retry_active);

        let warning = claimed_project("progress-warning", 91, 120);
        assert_eq!(
            recovery_progress_at(&warning, now, true).status,
            RecoveryProgressStatus::Warning
        );
        assert!(present_recovery(&warning).background_retry_active);

        let stalled = claimed_project("progress-stalled", 301, 400);
        assert_eq!(
            recovery_progress_at(&stalled, now, true).status,
            RecoveryProgressStatus::Stalled
        );
        assert!(present_recovery(&stalled).background_retry_active);

        let mut waiting = claimed_project("progress-waiting-human", 10, 30);
        waiting.workflow_state.recovery_state = Some(RecoveryState {
            phase: RecoveryPhase::WaitingHuman,
            error_kind: RecoveryErrorKind::HumanRequired,
            baseline_status: crate::project::RecoveryBaselineStatus::Unknown,
            subtask_id: "sub-1".to_string(),
            updated_at: now.to_rfc3339(),
            ..RecoveryState::default()
        });
        let waiting_autopilot = waiting
            .workflow_state
            .autopilot_state
            .as_mut()
            .expect("autopilot");
        waiting_autopilot.run_status = crate::project::AutopilotRunStatus::ErrorStopped;
        waiting_autopilot.recovery_action = AutopilotRecoveryAction::WaitHumanDecision;
        assert_eq!(
            recovery_progress_at(&waiting, now, false).status,
            RecoveryProgressStatus::WaitingHuman
        );
        let waiting_view = present_recovery(&waiting);
        assert!(!waiting_view.background_retry_active);
        assert!(waiting_view.heartbeat_status.starts_with("已停止"));
        assert_eq!(waiting_view.code_impact_summary, "执行基线恢复结果未记录。");

        let recovery = waiting
            .workflow_state
            .recovery_state
            .as_mut()
            .expect("recovery");
        recovery.baseline_status = crate::project::RecoveryBaselineStatus::Restored;
        recovery.baseline_stash_created = true;
        assert_eq!(
            present_recovery(&waiting).code_impact_summary,
            "未提交改动已暂存，工作区已恢复到执行基线。"
        );
        waiting
            .workflow_state
            .recovery_state
            .as_mut()
            .expect("recovery")
            .baseline_status = crate::project::RecoveryBaselineStatus::RestoreFailed;
        assert_eq!(
            present_recovery(&waiting).code_impact_summary,
            "执行基线恢复失败，需要人工检查工作区。"
        );

        let inactive = present_recovery(&Project::new("progress-inactive"));
        assert_eq!(inactive.progress_status, RecoveryProgressStatus::Inactive);
        assert!(!inactive.background_retry_active);
    }

    #[test]
    fn recovery_progress_dto_is_backward_compatible() {
        let project = Project::new("recovery-progress-compatibility");
        let presentation = present_recovery(&project);
        let mut legacy = serde_json::to_value(&presentation).expect("serialize presentation");
        let object = legacy.as_object_mut().expect("presentation object");
        for field in [
            "progress_status",
            "current_action",
            "action_started_at",
            "last_progress_at",
            "elapsed_seconds",
            "warning_at",
            "hard_deadline_at",
        ] {
            object.remove(field);
        }

        let restored: RecoveryPresentation =
            serde_json::from_value(legacy).expect("legacy presentation remains readable");
        assert_eq!(restored.progress_status, RecoveryProgressStatus::Inactive);
        assert!(restored.current_action.is_none());
        assert!(restored.action_started_at.is_none());
        assert!(restored.last_progress_at.is_none());
        assert!(restored.elapsed_seconds.is_none());
        assert!(restored.warning_at.is_none());
        assert!(restored.hard_deadline_at.is_none());

        let current = serde_json::to_value(presentation).expect("serialize current presentation");
        assert_eq!(current["progress_status"], "inactive");
    }
}
