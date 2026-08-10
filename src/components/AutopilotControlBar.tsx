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
import { useState, type ReactNode } from "react";
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
  resolveManagedActionSlots,
  type AutopilotActionId,
  type AutopilotActionSlots,
  type AutopilotBarState,
} from "../autopilotBarPresentation";
import {
  RecoveryDecisionDialog,
  type RecoveryDecisionSubmission,
} from "./RecoveryDecisionDialog";

export interface AutopilotControlBarProps {
  project: Project;
  recoveryPresentation: RecoveryPresentation | null;
  executionStatus?: PipelineState | null;
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
}: AutopilotBarShellProps) {
  return (
    <div
      className={`autopilot-control-bar ${className}`.trim()}
      data-ap-state={state}
      data-action-layout="fixed-slots"
      data-detail-layout="flow-bounded"
      data-recovery-kind={recoveryKind}
      data-recovery-fingerprint={recoveryFingerprint}
    >
      <div className="ap-bar-status-region" role="status" aria-label="自动驾驶状态">{status}</div>
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

export function AutopilotControlBar({
  project,
  recoveryPresentation,
  executionStatus,
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
  const heartbeatStale = isHeartbeatStale(
    autopilot?.heartbeat_at,
    autopilotActive && runStatus === "Running",
  );
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
    const enabled = action.enabled && handler !== null && (!writeDisabled || isSyncAction);
    const disabledReason = action.disabled_reason
      ?? (writeDisabled && !isSyncAction ? writeDisabledReason : undefined)
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
    const recoverySummary = compactAutopilotSummary([
      recovery.affected_task_label ? `任务：${recovery.affected_task_label}` : null,
      recovery.phase_label ? `阶段：${recovery.phase_label}` : null,
    ], recovery.title || "需要处理恢复状态");
    const recoverySecondaryActions: ReactNode[] = [];
    for (const action of recovery.secondary_actions) {
      const rendered = renderRecoveryAction(action, false);
      if (rendered) recoverySecondaryActions.push(rendered);
    }
    const recoveryActionSlots = partitionAutopilotActions<ReactNode>(
      recovery.primary_action ? renderRecoveryAction(recovery.primary_action, true) : null,
      recoverySecondaryActions,
    );
    return (
      <>
        <AutopilotBarShell
          state={recovery.severity === "Error" ? "Error" : "Recovery"}
          className={recovery.severity === "Error" ? "ap-error" : ""}
          recoveryKind={recovery.kind}
          recoveryFingerprint={recovery.state_fingerprint}
          status={(
            <span className="ap-bar-status"><AlertTriangle size={16} /> {recovery.title}</span>
          )}
          summary={recoverySummary}
          details={(
            <>
            {recovery.reason && <span className="ap-bar-error" title={recovery.reason}>{recovery.reason}</span>}
            {recovery.affected_task_label && <span className="ap-bar-target">任务：{recovery.affected_task_label}</span>}
            {recovery.phase_label && <span className="ap-bar-action">阶段：{recovery.phase_label}</span>}
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
            {recovery.background_retry_summary && (
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
      const managedActionSlots = renderActionSlots(
        resolveManagedActionSlots(
          presentation.canPause && !!onPauseManagedFlow,
          presentation.canResume && !!onResumeManagedFlow,
        ),
        actionId => {
          switch (actionId) {
            case "pause-managed":
              return (
                <button key={actionId} className="ap-bar-btn ap-bar-btn-primary" disabled={busy || writeDisabled} title={writeDisabled ? writeDisabledReason : undefined} onClick={onPauseManagedFlow}>
                  <Pause size={14} /> 暂停托管
                </button>
              );
            case "resume-managed":
              return (
                <button key={actionId} className="ap-bar-btn ap-bar-btn-primary" disabled={busy || writeDisabled} title={writeDisabled ? writeDisabledReason : undefined} onClick={onResumeManagedFlow}>
                  <Play size={14} /> {presentation.resumeLabel}
                </button>
              );
            case "stop-managed":
              return (
                <button key={actionId} className="ap-bar-btn" disabled={busy || writeDisabled} title={writeDisabled ? writeDisabledReason : undefined} onClick={onStopManagedFlow}>
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
          state={managedState}
          status={(
            <span className="ap-bar-status"><WandSparkles size={16} /> {presentation.statusLabel}</span>
          )}
          summary={compactAutopilotSummary([
            `目标：${presentation.targetLabel}`,
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
            </>
          )}
          actions={managedActionSlots}
        />
      );
    }
    if (project.workflow_state.top_level_phase !== "Console") return null;
    return (
      <AutopilotBarShell
        state="Waiting"
        status={<span className="ap-bar-status"><Play size={16} /> {canActivate ? "自动驾驶未激活" : "请先完成大阶段批准"}</span>}
        summary={canActivate ? "已具备激活条件，等待用户启动" : "完成当前批准步骤后可激活"}
        actions={{
          primary: <button
            className="ap-bar-btn ap-bar-btn-primary"
            disabled={busy || writeDisabled || !canActivate}
            title={writeDisabled ? writeDisabledReason : undefined}
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
  const runtimeActionSlots = renderActionSlots(
    runtimePresentation.actions,
    actionId => {
      switch (actionId) {
        case "pause-now":
          return (
            <button
              key={actionId}
              className={`ap-bar-btn ${isExecuting ? "ap-bar-btn-danger" : "ap-bar-btn-primary"}`}
              disabled={busy || writeDisabled}
              title={writeDisabled ? writeDisabledReason : undefined}
              onClick={onPauseNow}
            >
              {isExecuting ? <Square size={14} /> : <Pause size={14} />}
              {isExecuting ? "立即暂停" : "暂停自动驾驶"}
            </button>
          );
        case "pause-after-current":
          return (
            <button key={actionId} className="ap-bar-btn" disabled={busy || writeDisabled} title={writeDisabled ? writeDisabledReason : undefined} onClick={onPauseAfterCurrent}>
              <Pause size={14} /> 完成后暂停
            </button>
          );
        case "resume":
          return (
            <button key={actionId} className="ap-bar-btn ap-bar-btn-primary" disabled={busy || writeDisabled} title={writeDisabled ? writeDisabledReason : undefined} onClick={onResume}>
              <Play size={14} /> 恢复
            </button>
          );
        case "close":
          return (
            <button key={actionId} className="ap-bar-btn" disabled={busy || writeDisabled} title={writeDisabled ? writeDisabledReason : undefined} onClick={() => { void onToggle(false); }}>
              <Square size={14} /> 关闭
            </button>
          );
        default:
          return null;
      }
    },
  );
  return (
    <AutopilotBarShell
      state={runtimePresentation.state}
      className={runtimePresentation.state === "Running" ? "ap-running" : runtimePresentation.state === "Error" ? "ap-error" : ""}
      status={(
        <span className="ap-bar-status">
          {runStatus === "Running" || isExecuting ? <WandSparkles size={16} /> : <Pause size={16} />}
          {" "}{runtimePresentation.statusLabel}
        </span>
      )}
      summary={runtimePresentation.summary}
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
        </>
      )}
      actions={runtimeActionSlots}
    />
  );
}
