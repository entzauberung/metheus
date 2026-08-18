import {
  Activity,
  AlertTriangle,
  Clock3,
  FileQuestion,
  MoreHorizontal,
  Pause,
  Play,
  RotateCcw,
  ScanSearch,
  Square,
  TestTube2,
  WandSparkles,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type {
  PipelineState,
  Project,
  RecoveryActionPresentation,
  RecoveryCapability,
  RecoveryDecisionResolution,
  RecoveryPresentation,
} from "../types";
import {
  getQualityStatusPresentation,
  getVerificationStageLabel,
  isHeartbeatStale,
} from "../autopilotPolicy";
import { getManagedFlowPresentation } from "../managedFlowPolicy";
import { findProjectSubtaskById, isSubtaskLeaf } from "../taskTreePolicy";
import {
  compactAutopilotSummary,
  partitionAutopilotActions,
  resolveAutopilotRuntimePresentation,
  resolveRecoveryBarProgress,
  resolveManagedActionSlots,
  type AutopilotActionId,
  type AutopilotActionSlots,
  type AutopilotBarState,
} from "../autopilotBarPresentation";
import {
  RecoveryDecisionDialog,
  type RecoveryDecisionSubmission,
} from "./RecoveryDecisionDialog";
import type { RuntimeOutcomePresentation } from "../runtimeOutcomePresentation";

export interface AutopilotControlBarProps {
  project: Project;
  recoveryPresentation: RecoveryPresentation | null;
  executionStatus?: PipelineState | null;
  runtimeOutcome?: RuntimeOutcomePresentation;
  busy: boolean;
  writeDisabled?: boolean;
  writeDisabledReason?: string;
  onToggle: (active: boolean) => Promise<void>;
  onPauseManagedFlow?: () => Promise<void>;
  onResumeManagedFlow?: () => Promise<void>;
  onStopManagedFlow: () => Promise<void>;
  onPauseNow: () => Promise<void>;
  onPauseAfterCurrent: () => Promise<void>;
  onResume: () => Promise<void>;
  onSync: () => Promise<void>;
  onAcknowledgeRecovery?: () => Promise<void>;
  onRegeneratePlan?: () => Promise<void>;
  onPrepareWorkspace?: () => Promise<void>;
  onRefreshWorkspace?: () => Promise<void>;
  onRetryGitConfirmation?: () => Promise<void>;
  onRunAutomaticRecovery?: () => Promise<void>;
  onResolveHumanRecovery?: (
    resolution: RecoveryDecisionResolution,
    reason: string,
    acceptedCriteria: number[],
  ) => Promise<void>;
}

const AUTOPILOT_ACTION_LABELS: Record<string, string> = {
  select_milestone: "选择大阶段",
  transition_workflow: "切换工作流",
  generate_mid_stage_draft: "生成中阶段草稿",
  regenerate_mid_stage_draft: "重生成中阶段草稿",
  check_mid_stage_draft: "检查中阶段草稿",
  approve_mid_stage_draft: "批准中阶段草稿",
  select_mid_stage: "选择中阶段",
  generate_execution_plan: "生成执行计划",
  regenerate_execution_plan: "重生成执行计划",
  check_stage_plan: "检查执行计划",
  approve_stage_plan: "批准执行计划",
  calibrate_next_subtask_command: "校准下一任务",
  execute_current_subtask: "执行当前任务",
  confirm_subtask_result: "确认任务结果",
  run_error_recovery: "恢复质量错误",
  prepare_execution_workspace: "准备 Git 工作区",
  refresh_execution_workspace: "刷新 Git 工作区",
};

interface AutopilotBarShellProps {
  state: AutopilotBarState;
  className?: string;
  status: ReactNode;
  summary: string;
  details?: ReactNode;
  actions: AutopilotActionSlots<ReactNode>;
  recoveryKind?: string;
  recoveryFingerprint?: string;
  recoveryProgress?: string;
  statusAriaLabel?: string;
}

function AutopilotBarActionSlots({ actions }: { actions: AutopilotActionSlots<ReactNode> }) {
  return (
    <>
      <div className="ap-action-slot" data-action-slot="primary">{actions.primary}</div>
      <div className="ap-action-slot" data-action-slot="secondary">{actions.secondary}</div>
      <div className="ap-action-slot" data-action-slot="overflow">
        {actions.overflow.length > 0 && (
          <details className="ap-action-overflow">
            <summary aria-label="更多自动驾驶操作">
              <MoreHorizontal size={15} aria-hidden="true" />
              <span>更多</span>
            </summary>
            <div className="ap-action-overflow-menu">
              {actions.overflow}
            </div>
          </details>
        )}
      </div>
    </>
  );
}

function renderActionSlots(
  actions: AutopilotActionSlots,
  renderAction: (action: AutopilotActionId) => ReactNode,
): AutopilotActionSlots<ReactNode> {
  return {
    primary: actions.primary ? renderAction(actions.primary) : null,
    secondary: actions.secondary ? renderAction(actions.secondary) : null,
    overflow: actions.overflow.map(renderAction),
  };
}

function AutopilotBarShell({
  state,
  className = "",
  status,
  summary,
  details,
  actions,
  recoveryKind,
  recoveryFingerprint,
  recoveryProgress,
  statusAriaLabel = "自动驾驶状态",
}: AutopilotBarShellProps) {
  return (
    <div
      className={`autopilot-control-bar ${className}`.trim()}
      data-ap-state={state}
      data-action-layout="fixed-slots"
      data-detail-layout="flow-bounded"
      data-recovery-kind={recoveryKind}
      data-recovery-fingerprint={recoveryFingerprint}
      data-recovery-progress={recoveryProgress}
    >
      <div
        className="ap-bar-status-region"
        role="status"
        aria-label={statusAriaLabel}
        aria-live="polite"
        aria-atomic="true"
      >
        {status}
      </div>
      <div className="ap-bar-summary-region" role="group" aria-label="执行摘要">
        <span className="ap-bar-summary" title={summary}>{summary}</span>
        {details && (
          <details className="ap-bar-details">
            <summary>查看详情</summary>
            <div className="ap-bar-detail-content">{details}</div>
          </details>
        )}
      </div>
      <div className="ap-bar-actions-region ap-bar-right" role="group" aria-label="自动驾驶操作">
        <AutopilotBarActionSlots actions={actions} />
      </div>
    </div>
  );
}

function formatStateTime(value: string | undefined): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function formatElapsedSeconds(value: number | null | undefined): string {
  if (value === null || value === undefined) return "";
  const seconds = Math.max(0, Math.floor(value));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return remainder > 0 ? `${minutes} 分 ${remainder} 秒` : `${minutes} 分`;
}

function recoveryProgressIcon(status: ReturnType<typeof resolveRecoveryBarProgress>["status"]) {
  switch (status) {
    case "queued":
    case "scheduled":
      return <Clock3 size={16} aria-hidden="true" />;
    case "running":
      return <Activity size={16} aria-hidden="true" />;
    case "warning":
    case "stalled":
      return <AlertTriangle size={16} aria-hidden="true" />;
    case "waiting_human":
      return <Square size={15} aria-hidden="true" />;
    case "inactive":
    case "unknown":
      return <FileQuestion size={16} aria-hidden="true" />;
  }
}

export function AutopilotControlBar({
  project,
  recoveryPresentation,
  executionStatus,
  runtimeOutcome,
  busy,
  writeDisabled = false,
  writeDisabledReason = "",
  onToggle,
  onPauseManagedFlow,
  onResumeManagedFlow,
  onStopManagedFlow,
  onPauseNow,
  onPauseAfterCurrent,
  onResume,
  onSync,
  onAcknowledgeRecovery,
  onRegeneratePlan,
  onPrepareWorkspace,
  onRefreshWorkspace,
  onRetryGitConfirmation,
  onRunAutomaticRecovery,
  onResolveHumanRecovery,
}: AutopilotControlBarProps) {
  const [decisionOpen, setDecisionOpen] = useState(false);
  const recovery = recoveryPresentation?.kind !== "None" ? recoveryPresentation : null;
  const autopilotActive = project.workflow_state.autopilot_active === true;
  const autopilot = project.workflow_state.autopilot_state;
  const managed = project.workflow_state.managed_flow_state;
  const isExecuting = executionStatus?.status === "Running";
  const runStatus = autopilot?.run_status ?? "Paused";
  const currentAction = autopilot?.current_action_kind
    ? AUTOPILOT_ACTION_LABELS[autopilot.current_action_kind] ?? autopilot.current_action_kind
    : "";
  const targetMilestone = project.milestones.find(
    milestone => milestone.id === project.workflow_state.autopilot_target_milestone_id,
  );
  const targetLabel = targetMilestone?.title ?? project.workflow_state.autopilot_target_milestone_id;
  const retryCount = autopilot?.transient_retry_count ?? 0;
  const retryAt = formatStateTime(autopilot?.next_retry_at);
  const validationRetryAt = formatStateTime(
    project.workflow_state.recovery_state?.next_validation_retry_at,
  );
  const validationStage = project.execution_session?.verification_stage;
  const heartbeatAt = formatStateTime(autopilot?.heartbeat_at);
  const autopilotHeartbeatStale = isHeartbeatStale(
    autopilot?.heartbeat_at,
    autopilotActive && runStatus === "Running",
  );
  const managedHeartbeatStale = isHeartbeatStale(
    managed?.heartbeat_at,
    managed?.active === true && managed.run_status === "Running",
  );
  const heartbeatStale = autopilotHeartbeatStale || managedHeartbeatStale;
  const staleHeartbeatSyncRef = useRef<string | null>(null);
  useEffect(() => {
    const heartbeat = autopilotHeartbeatStale
      ? `autopilot:${autopilot?.heartbeat_at ?? ""}`
      : managedHeartbeatStale
        ? `managed:${managed?.heartbeat_at ?? ""}`
        : "";
    if (!heartbeatStale || !heartbeat) {
      if (!heartbeatStale) staleHeartbeatSyncRef.current = null;
      return;
    }
    if (busy || runtimeOutcome?.syncStatus === "syncing"
      || staleHeartbeatSyncRef.current === heartbeat) return;
    staleHeartbeatSyncRef.current = heartbeat;
    void onSync();
  }, [autopilot?.heartbeat_at, autopilotHeartbeatStale, busy, heartbeatStale,
    managed?.heartbeat_at, managedHeartbeatStale, onSync, runtimeOutcome?.syncStatus]);
  const effectiveWriteDisabled = writeDisabled || runtimeOutcome?.writeAllowed === false;
  const effectiveWriteDisabledReason = writeDisabledReason || runtimeOutcome?.writeBlockedReason || "";
  const autopilotOwnerMissing = autopilotActive && runStatus === "Running"
    && (autopilot?.job_owner !== "BackendRuntime" || !autopilot?.job_id);
  const managedOwnerMissing = managed?.active === true
    && managed.run_status === "Running"
    && !managed.job_id;
  const safetyLimited = effectiveWriteDisabled || heartbeatStale || autopilotOwnerMissing || managedOwnerMissing;
  const safetyLimitedReason = effectiveWriteDisabledReason
    || (autopilotOwnerMissing || managedOwnerMissing
      ? "后台 owner 未被证明，请先同步或执行停止/人工处理"
      : heartbeatStale ? "后台心跳已陈旧，请先同步状态" : "");
  const recoveryTaskId = !recovery ? project.execution_session?.subtask_id ?? "" : "";
  const recoveryTaskCandidate = findProjectSubtaskById(project, recoveryTaskId);
  const recoveryTask = recoveryTaskCandidate && isSubtaskLeaf(recoveryTaskCandidate)
    ? recoveryTaskCandidate
    : null;
  const qualityStatuses = recoveryTask
    ? getQualityStatusPresentation(
      recoveryTask.test_result,
      recoveryTask.acceptance_ledger ?? [],
    )
    : [];

  const recoveryHandler = (capability: RecoveryCapability): (() => Promise<void>) | null => {
    switch (capability) {
      case "SyncProject": return onSync;
      case "ClearStaleControlLock": return onSync;
      case "AcknowledgeExecutionRecovery": return onAcknowledgeRecovery ?? null;
      case "RetryGitConfirmation": return onRetryGitConfirmation ?? null;
      case "RetryAutopilotAdvance":
      case "ResumeAutopilot": return onResume;
      case "RegenerateExecutionPlan": return onRegeneratePlan ?? null;
      case "PrepareExecutionWorkspace": return onPrepareWorkspace ?? null;
      case "RefreshExecutionWorkspace": return onRefreshWorkspace ?? null;
      case "RunAutomaticRecovery": return onRunAutomaticRecovery ?? null;
      case "ResolveHumanRecovery": return project.workflow_state.recovery_state
        && onResolveHumanRecovery && recovery?.decision_options.some(option => option.enabled)
        ? async () => { setDecisionOpen(true); }
        : null;
      case "CloseAutopilot": return () => onToggle(false);
    }
  };

  const renderRecoveryAction = (
    action: RecoveryActionPresentation,
    primary: boolean,
  ) => {
    if (!recovery?.capabilities.includes(action.capability)) return null;
    if (action.capability === "ResolveHumanRecovery" && !project.workflow_state.recovery_state) {
      return null;
    }
    const handler = recoveryHandler(action.capability);
    const isSyncAction = action.capability === "SyncProject"
      || action.capability === "ClearStaleControlLock";
    const isSafeAction = isSyncAction
      || action.capability === "CloseAutopilot"
      || action.capability === "ResolveHumanRecovery";
    const enabled = action.enabled && handler !== null && (!safetyLimited || isSafeAction);
    const disabledReason = action.disabled_reason
      ?? (safetyLimited && !isSafeAction ? safetyLimitedReason : undefined)
      ?? (!handler ? "当前界面未连接此恢复动作" : undefined);
    return (
      <button
        key={`${primary ? "primary" : "secondary"}-${action.capability}`}
        className={`ap-bar-btn ${primary ? "ap-bar-btn-primary" : ""}`}
        disabled={busy || !enabled}
        title={disabledReason}
        onClick={() => { if (handler) void handler(); }}
      >
        <RotateCcw size={14} /> {action.label}
      </button>
    );
  };

  if (recovery) {
    const progress = resolveRecoveryBarProgress(recovery);
    const recoveryAction = recovery.current_action
      ? AUTOPILOT_ACTION_LABELS[recovery.current_action] ?? recovery.current_action
      : "";
    const elapsed = formatElapsedSeconds(recovery.elapsed_seconds);
    const lastProgressAt = formatStateTime(recovery.last_progress_at ?? undefined);
    const nextRecoveryAt = formatStateTime(
      recovery.next_validation_retry_at ?? recovery.next_retry_at ?? undefined,
    );
    const progressHasBackground = recovery.background_retry_active === true && (
      progress.status === "scheduled"
      || progress.status === "running"
      || progress.status === "warning"
      || progress.status === "stalled"
    );
    const recoverySummary = compactAutopilotSummary([
      recovery.affected_task_label ? `任务：${recovery.affected_task_label}` : null,
      runtimeOutcome ? `${runtimeOutcome.statusLabel}：${runtimeOutcome.summary}` : null,
      progress.summary,
      recoveryAction ? `动作：${recoveryAction}` : null,
      elapsed ? `已持续：${elapsed}` : null,
      progress.status === "scheduled" && nextRecoveryAt ? `下次重试：${nextRecoveryAt}` : null,
      (progress.status === "warning" || progress.status === "stalled") && lastProgressAt
        ? `最后进展：${lastProgressAt}`
        : null,
    ], progress.summary);
    const recoverySecondaryActions: ReactNode[] = [];
    for (const action of recovery.secondary_actions) {
      const rendered = renderRecoveryAction(action, false);
      if (rendered) recoverySecondaryActions.push(rendered);
    }
    const recoveryActionSlots = partitionAutopilotActions<ReactNode>(
      recovery.primary_action ? renderRecoveryAction(recovery.primary_action, true) : null,
      [
        ...recoverySecondaryActions,
        ...(safetyLimited && autopilotActive && !recovery.capabilities.includes("CloseAutopilot")
          ? [
            <button
              key="safety-close-autopilot"
              className="ap-bar-btn"
              disabled={busy}
              title={safetyLimitedReason || undefined}
              onClick={() => { void onToggle(false); }}
            >
              <Square size={14} /> 关闭自动驾驶
            </button>,
          ]
          : []),
        ...(safetyLimited && managed?.active && onStopManagedFlow
          ? [
            <button
              key="safety-stop-managed"
              className="ap-bar-btn"
              disabled={busy}
              title={safetyLimitedReason || undefined}
              onClick={() => { void onStopManagedFlow(); }}
            >
              <Square size={14} /> 停止托管
            </button>,
          ]
          : []),
      ],
    );
    const recoveryIsError = progress.tone === "error"
      || (progress.status === "unknown" && recovery.severity === "Error");
    return (
      <>
        <AutopilotBarShell
          state={recoveryIsError ? "Error" : "Recovery"}
          className={recoveryIsError ? "ap-error" : ""}
          recoveryKind={recovery.kind}
          recoveryFingerprint={recovery.state_fingerprint}
          recoveryProgress={progress.status}
          statusAriaLabel="恢复进展"
          status={(
            <span className="ap-bar-status">
              {recoveryProgressIcon(progress.status)}
              {runtimeOutcome?.statusLabel ?? progress.statusLabel}
            </span>
          )}
          summary={recoverySummary}
          details={(
            <>
            {recovery.reason && <span className="ap-bar-error" title={recovery.reason}>{recovery.reason}</span>}
            {recovery.affected_task_label && <span className="ap-bar-target">任务：{recovery.affected_task_label}</span>}
            {recovery.phase_label && <span className="ap-bar-action">阶段：{recovery.phase_label}</span>}
            {recoveryAction && <span className="ap-bar-action ap-recovery-fact">动作：{recoveryAction}</span>}
            {recovery.action_started_at && (
              <span className="ap-bar-target">开始：{formatStateTime(recovery.action_started_at)}</span>
            )}
            {elapsed && <span className="ap-bar-target">已持续：{elapsed}</span>}
            {lastProgressAt && <span className="ap-bar-target">最后业务进展：{lastProgressAt}</span>}
            {recovery.warning_at && (
              <span className="ap-bar-warning ap-recovery-fact">
                进展警告时间：{formatStateTime(recovery.warning_at)}
              </span>
            )}
            {recovery.hard_deadline_at && (
              <span className="ap-bar-warning ap-recovery-fact">
                最迟终止：{formatStateTime(recovery.hard_deadline_at)}
              </span>
            )}
            {progress.status === "scheduled" && nextRecoveryAt && (
              <span className="ap-bar-action ap-recovery-fact">下次重试：{nextRecoveryAt}</span>
            )}
            {recovery.validation_phase_label && <span className="ap-bar-action">验证：{recovery.validation_phase_label}</span>}
            {recovery.retry_count !== undefined && recovery.retry_count > 0 && (
              <span className="ap-bar-warning">
                后台重试 {recovery.retry_count}/{recovery.retry_limit ?? 0}
                {recovery.next_retry_at ? ` · ${formatStateTime(recovery.next_retry_at)}` : ""}
              </span>
            )}
            {recovery.validation_retry_count !== undefined && recovery.validation_retry_count > 0 && (
              <span className="ap-bar-warning">
                验证重试 {recovery.validation_retry_count}/{recovery.validation_retry_limit ?? 0}
                {recovery.next_validation_retry_at ? ` · ${formatStateTime(recovery.next_validation_retry_at)}` : ""}
              </span>
            )}
            {recovery.heartbeat_status && <span className="ap-bar-target">心跳：{recovery.heartbeat_status}</span>}
            {recovery.control_action_description && (
              <span className="ap-bar-target">占用：{recovery.control_action_description}</span>
            )}
            {recovery.control_action_elapsed_seconds !== undefined
              && recovery.kind === "ControlActionOccupied" && (
                <span className="ap-bar-target">已持续：{recovery.control_action_elapsed_seconds} 秒</span>
              )}
            {recovery.control_lock_failure_reason && (
              <span className="ap-bar-warning">失效原因：{recovery.control_lock_failure_reason}</span>
            )}
            {[recovery.automated_test_status, recovery.code_review_status,
              recovery.review_protocol_status, recovery.acceptance_evidence_status]
              .filter(Boolean)
              .map(status => <span key={status} className="ap-bar-warning">{status}</span>)}
            {recovery.code_impact_summary && (
              <span className="ap-bar-warning">{recovery.code_impact_summary}</span>
            )}
            {progressHasBackground && recovery.background_retry_summary && (
              <span className="ap-bar-action">{recovery.background_retry_summary}</span>
            )}
            {recovery.post_action_expectation && (
              <span className="ap-bar-hint">动作后：{recovery.post_action_expectation}</span>
            )}
            {recovery.sync_risk_summary && (
              <span className="ap-bar-warning">{recovery.sync_risk_summary}</span>
            )}
            {recovery.primary_action && !recovery.primary_action.enabled && recovery.primary_action.disabled_reason && (
              <span className="ap-bar-hint">{recovery.primary_action.disabled_reason}</span>
            )}
            {runtimeOutcome && (
              <span className="ap-bar-action">
                运行结果：{runtimeOutcome.statusLabel}；执行 {runtimeOutcome.execution}；质量 {runtimeOutcome.quality}；验收 {runtimeOutcome.acceptance}
              </span>
            )}
            {runtimeOutcome && !runtimeOutcome.writeAllowed && (
              <span className="ap-bar-warning">{runtimeOutcome.writeBlockedReason}</span>
            )}
            </>
          )}
          actions={recoveryActionSlots}
        />
        <RecoveryDecisionDialog
          isOpen={decisionOpen}
          project={project}
          presentation={recovery}
          busy={busy}
          onClose={() => setDecisionOpen(false)}
          onSubmit={async ({ resolution, reason, acceptedCriteria }: RecoveryDecisionSubmission) => {
            if (!onResolveHumanRecovery) return;
            await onResolveHumanRecovery(resolution, reason, acceptedCriteria);
            setDecisionOpen(false);
          }}
        />
      </>
    );
  }

  const activationSteps = new Set([
    "MilestoneSelection", "MidStageGeneration", "MidStageCheck", "MidStageApproval",
    "MidStageSelection", "PlanGeneration", "PlanCheck", "PlanApproving", "Execution",
  ]);
  const canActivate = activationSteps.has(project.workflow_state.current_step);

  if (!autopilotActive) {
    if (managed?.active) {
      const presentation = getManagedFlowPresentation(
        managed,
        project.workflow_state.current_step,
        project.milestone_draft,
      );
      const managedState: AutopilotBarState = managed.run_status === "Running"
        ? "Running"
        : managed.run_status === "Paused"
          ? "Paused"
          : "Waiting";
      const managedActionSlots = safetyLimited
        ? {
          primary: <button key="sync-managed" className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onSync}>
            <RotateCcw size={14} /> 同步状态
          </button>,
          secondary: <button key="stop-managed-safe" className="ap-bar-btn" disabled={busy} onClick={onStopManagedFlow}>
            <Square size={14} /> 停止托管
          </button>,
          overflow: [],
        }
        : renderActionSlots(
          resolveManagedActionSlots(
            presentation.canPause && !!onPauseManagedFlow,
            presentation.canResume && !!onResumeManagedFlow,
          ),
          actionId => {
          switch (actionId) {
            case "pause-managed":
              return (
                  <button key={actionId} className="ap-bar-btn ap-bar-btn-primary" disabled={busy || effectiveWriteDisabled} title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined} onClick={onPauseManagedFlow}>
                  <Pause size={14} /> 暂停托管
                </button>
              );
            case "resume-managed":
              return (
                <button key={actionId} className="ap-bar-btn ap-bar-btn-primary" disabled={busy || effectiveWriteDisabled} title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined} onClick={onResumeManagedFlow}>
                  <Play size={14} /> {presentation.resumeLabel}
                </button>
              );
            case "stop-managed":
              return (
                <button key={actionId} className="ap-bar-btn" disabled={busy || effectiveWriteDisabled} title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined} onClick={onStopManagedFlow}>
                  <Square size={14} /> 停止托管
                </button>
              );
            default:
              return null;
          }
          },
        );
      return (
        <AutopilotBarShell
          state={runtimeOutcome?.tone === "error" ? "Error" : managedState}
          status={(
            <span className="ap-bar-status"><WandSparkles size={16} /> {runtimeOutcome?.statusLabel ?? presentation.statusLabel}</span>
          )}
          summary={compactAutopilotSummary([
            `目标：${presentation.targetLabel}`,
            runtimeOutcome?.summary ?? null,
          ], "托管流程等待后端状态")}
          details={(
            <>
            {managed.last_action && <span className="ap-bar-action">{managed.last_action}</span>}
            <span className="ap-bar-target">目标：{presentation.targetLabel}</span>
            <span className="ap-bar-target">动作：{presentation.actionLabel}</span>
            <span className="ap-bar-target">心跳：{presentation.heartbeatLabel}</span>
            {presentation.detail && presentation.detail !== managed.last_action && (
              <span className="ap-bar-error" title={presentation.detail}>{presentation.detail}</span>
            )}
            {runtimeOutcome && !runtimeOutcome.writeAllowed && (
              <span className="ap-bar-warning">{runtimeOutcome.writeBlockedReason}</span>
            )}
            {managedHeartbeatStale && <span className="ap-bar-error">心跳异常：托管状态可能已停止更新</span>}
            </>
          )}
          actions={managedActionSlots}
        />
      );
    }
    if (project.workflow_state.top_level_phase !== "Console") return null;
    if (safetyLimited) {
      return (
        <AutopilotBarShell
          state="Error"
          status={<span className="ap-bar-status"><AlertTriangle size={16} /> 状态需对账</span>}
          summary={safetyLimitedReason || "当前仅允许同步状态或停止后台控制"}
          actions={{
            primary: <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onSync}>
              <RotateCcw size={14} /> 同步状态
            </button>,
            secondary: null,
            overflow: [],
          }}
        />
      );
    }
    return (
      <AutopilotBarShell
        state="Waiting"
        status={<span className="ap-bar-status"><Play size={16} /> {runtimeOutcome?.statusLabel ?? (canActivate ? "自动驾驶未激活" : "请先完成大阶段批准")}</span>}
        summary={runtimeOutcome?.summary ?? (canActivate ? "已具备激活条件，等待用户启动" : "完成当前批准步骤后可激活")}
        actions={{
          primary: <button
            className="ap-bar-btn ap-bar-btn-primary"
            disabled={busy || effectiveWriteDisabled || !canActivate}
            title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined}
            onClick={() => { void onToggle(true); }}
          >
            <WandSparkles size={14} /> 激活自动驾驶
          </button>,
          secondary: null,
          overflow: [],
        }}
      />
    );
  }

  const runtimePresentation = resolveAutopilotRuntimePresentation(
    runStatus,
    isExecuting,
    targetLabel,
  );
  const runtimeActionSlots = safetyLimited
    ? {
      primary: <button key="sync-autopilot" className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onSync}>
        <RotateCcw size={14} /> 同步状态
      </button>,
      secondary: <button key="close-autopilot-safe" className="ap-bar-btn" disabled={busy} onClick={() => { void onToggle(false); }}>
        <Square size={14} /> 关闭自动驾驶
      </button>,
      overflow: [],
    }
    : renderActionSlots(
      runtimePresentation.actions,
      actionId => {
      switch (actionId) {
        case "pause-now":
          return (
            <button
              key={actionId}
              className={`ap-bar-btn ${isExecuting ? "ap-bar-btn-danger" : "ap-bar-btn-primary"}`}
              disabled={busy || effectiveWriteDisabled}
              title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined}
              onClick={onPauseNow}
            >
              {isExecuting ? <Square size={14} /> : <Pause size={14} />}
              {isExecuting ? "立即暂停" : "暂停自动驾驶"}
            </button>
          );
        case "pause-after-current":
          return (
            <button key={actionId} className="ap-bar-btn" disabled={busy || effectiveWriteDisabled} title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined} onClick={onPauseAfterCurrent}>
              <Pause size={14} /> 完成后暂停
            </button>
          );
        case "resume":
          return (
            <button key={actionId} className="ap-bar-btn ap-bar-btn-primary" disabled={busy || effectiveWriteDisabled} title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined} onClick={onResume}>
              <Play size={14} /> 恢复
            </button>
          );
        case "close":
          return (
            <button key={actionId} className="ap-bar-btn" disabled={busy || effectiveWriteDisabled} title={effectiveWriteDisabled ? effectiveWriteDisabledReason : undefined} onClick={() => { void onToggle(false); }}>
              <Square size={14} /> 关闭
            </button>
          );
        default:
          return null;
      }
      },
    );
  const activeBarState = runtimeOutcome?.tone === "error" ? "Error" : runtimePresentation.state;
  return (
    <AutopilotBarShell
      state={activeBarState}
      className={activeBarState === "Running" ? "ap-running" : activeBarState === "Error" ? "ap-error" : ""}
      status={(
        <span className="ap-bar-status">
          {runStatus === "Running" || isExecuting ? <WandSparkles size={16} /> : <Pause size={16} />}
          {" "}{runtimeOutcome?.statusLabel ?? runtimePresentation.statusLabel}
        </span>
      )}
      summary={runtimeOutcome?.summary ?? runtimePresentation.summary}
      details={(
        <>
        {targetLabel && <span className="ap-bar-target">目标：{targetLabel}</span>}
        {currentAction && <span className="ap-bar-action">当前：{currentAction}</span>}
        {retryAt && <span className="ap-bar-warning"><Clock3 size={13} /> 重试 {retryCount}/3 · {retryAt}</span>}
        {validationRetryAt && (
          <span className="ap-bar-warning">
            <Clock3 size={13} /> 验证重试 {project.workflow_state.recovery_state?.validation_retry_count ?? 0}/{project.workflow_state.recovery_state?.max_validation_retries ?? 0} · {validationRetryAt}
          </span>
        )}
        {validationStage && (
          <span className="ap-bar-action"><ScanSearch size={13} /> 验证阶段：{getVerificationStageLabel(validationStage)}</span>
        )}
        {heartbeatAt && <span className="ap-bar-target">心跳 {heartbeatAt}</span>}
        {heartbeatStale && <span className="ap-bar-error">心跳异常：后台状态可能已停止更新</span>}
        {qualityStatuses.map(status => (
          <span
            key={status.key}
            className="ap-bar-warning"
            style={{ color: status.tone === "success" ? "#1a7f37" : status.tone === "error" ? "#cf222e" : status.tone === "warning" ? "#9a6700" : "#656d76" }}
          >
            {status.key === "automated-test" ? <TestTube2 size={13} />
              : status.key === "code-review" ? <ScanSearch size={13} />
                : status.key === "review-protocol" ? <Activity size={13} />
                  : <FileQuestion size={13} />}
            {" "}{status.label}
          </span>
        ))}
        {autopilot?.last_action && <span className="ap-bar-target">{autopilot.last_action}</span>}
        {runtimeOutcome && (
          <span className="ap-bar-action">
            执行：{runtimeOutcome.execution}；质量：{runtimeOutcome.quality}；验收：{runtimeOutcome.acceptance}；确认：{runtimeOutcome.confirmation}
          </span>
        )}
        {runtimeOutcome && !runtimeOutcome.writeAllowed && (
          <span className="ap-bar-warning">{runtimeOutcome.writeBlockedReason}</span>
        )}
        </>
      )}
      actions={runtimeActionSlots}
    />
  );
}
