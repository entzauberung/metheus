// src/components/AutopilotControlBar.tsx — 全局自动驾驶控制条
// 在所有 Console 页面顶部显示自动驾驶状态与操作入口
import { Pause, Play, RotateCcw, Square, WandSparkles, AlertTriangle, GitBranch, CheckCircle, CircleSlash2, TestTube2, ScanSearch, FileQuestion, Activity, Clock3, Settings2 } from "lucide-react";
import type { Project, PipelineState, AutopilotRecoveryAction } from "../types";
import { getAutopilotErrorActions, getGitConfirmationBlockPresentation, getQualityStatusPresentation, getRecoveryStatusLabel, getVerificationStageLabel, isHeartbeatStale, isValidationRecovery } from "../autopilotPolicy";
import { getManagedFlowPresentation } from "../managedFlowPolicy";
import { findProjectSubtaskById, isSubtaskLeaf } from "../taskTreePolicy";

export interface AutopilotControlBarProps {
  project: Project;
  executionStatus?: PipelineState | null;
  busy: boolean;
  onToggle: (active: boolean) => Promise<void>;
  onStopManagedFlow: () => Promise<void>;
  onPauseNow: () => Promise<void>;
  onPauseAfterCurrent: () => Promise<void>;
  onResume: () => Promise<void>;
  onSync: () => Promise<void>;
  onRetryCurrent?: () => Promise<void>;
  onAcknowledgeRecovery?: () => Promise<void>;
  onRegeneratePlan?: () => Promise<void>;
  onPrepareWorkspace?: () => Promise<void>;
  onRefreshWorkspace?: () => Promise<void>;
  onRetryGitConfirmation?: () => Promise<void>;
  onResolveHumanRecovery?: (
    resolution: "retest" | "revalidate" | "restore_and_retry" | "regenerate_plan" | "confirm_actual_pass" | "accept_deviation" | "skip_task",
  ) => Promise<void>;
}

function sessionStatusKey(status: string | undefined): string {
  return (status ?? "").toLowerCase();
}

function isRecoverableSession(project: Project): boolean {
  const status = sessionStatusKey(project.execution_session?.status);
  return (
    status === "execution_failed"
    || status === "session_lost"
    || status === "stop_failed"
  );
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

function formatStateTime(value: string | undefined): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function AutopilotControlBar({
  project, executionStatus, busy,
  onToggle, onStopManagedFlow, onPauseNow, onPauseAfterCurrent, onResume, onSync,
  onRetryCurrent, onAcknowledgeRecovery, onRegeneratePlan, onPrepareWorkspace,
  onRefreshWorkspace, onRetryGitConfirmation, onResolveHumanRecovery,
}: AutopilotControlBarProps) {
  const apActive = project.workflow_state.autopilot_active === true;
  const apState = project.workflow_state.autopilot_state;
  const mfActive = project.workflow_state.managed_flow_state?.active === true;
  const mfState = project.workflow_state.managed_flow_state;
  const isExecuting = executionStatus?.status === "Running";
  const activationSteps = new Set([
    "MilestoneSelection", "MidStageGeneration", "MidStageCheck", "MidStageApproval",
    "MidStageSelection", "PlanGeneration", "PlanCheck", "PlanApproving", "Execution",
  ]);
  const canActivate = activationSteps.has(project.workflow_state.current_step);

  const runStatus = apState?.run_status;
  const lastAction = apState?.last_action;
  const currentAction = apState?.current_action_kind
    ? (AUTOPILOT_ACTION_LABELS[apState.current_action_kind] ?? apState.current_action_kind)
    : "";
  const retryCount = apState?.transient_retry_count ?? 0;
  const retryAt = formatStateTime(apState?.next_retry_at);
  const heartbeatAt = formatStateTime(apState?.heartbeat_at);
  const errorMessage = apState?.error_message;
  const recoveryAction: AutopilotRecoveryAction = apState?.recovery_action ?? "None";
  const recovery = project.workflow_state.recovery_state;
  const session = project.execution_session;
  const waitingEngine = recovery?.phase === "WaitingEngine" || recovery?.error_kind === "EngineBlocked";
  const transientFailureKinds = new Set([
    "Network", "RateLimited", "ProviderUnavailable", "Timeout", "RevisionConflict", "ProcessCrash",
  ]);
  const automaticRetryPending = retryCount > 0
    && Boolean(apState?.next_retry_at)
    && transientFailureKinds.has(apState?.last_failure_kind ?? "None");
  const validationRetryPending = recovery?.phase === "Retesting"
    && isValidationRecovery(recovery)
    && (recovery.validation_retry_count ?? 0) < (recovery.max_validation_retries ?? 0);
  const validationRetryAt = formatStateTime(recovery?.next_validation_retry_at);
  const validationStage = session?.verification_stage;
  const validating = Boolean(validationStage && !["NotStarted", "Completed"].includes(validationStage));
  const validationHumanBoundary = recovery?.phase === "WaitingHuman" && isValidationRecovery(recovery);
  const reviewServiceBlocked = recovery?.error_kind === "ReviewServiceBlocked";
  const heartbeatStale = isHeartbeatStale(
    apState?.heartbeat_at,
    apActive && runStatus === "Running" && !validationRetryPending,
  );
  const targetMs = project.milestones.find(m => m.id === project.workflow_state.autopilot_target_milestone_id);
  const targetLabel = targetMs?.title ?? project.workflow_state.autopilot_target_milestone_id;
  const targetSubtask = findProjectSubtaskById(
    project,
    recovery?.subtask_id ?? session?.subtask_id ?? "",
  );
  const recoverySubtask = targetSubtask && isSubtaskLeaf(targetSubtask)
    ? targetSubtask
    : null;
  const qualityStatuses = recoverySubtask
    ? getQualityStatusPresentation(
      recoverySubtask.test_result,
      recoverySubtask.acceptance_ledger ?? [],
    )
    : [];

  // 先判断失败会话，再判断自动驾驶是否激活 — 手动模式也要看到恢复入口
  const sessionKey = sessionStatusKey(session?.status);
  const sessionLost = sessionKey === "session_lost";
  const stopFailed = sessionKey === "stop_failed";
  const confirmationBlocked = sessionKey === "confirmation_blocked";
  const confirmationPresentation = getGitConfirmationBlockPresentation(
    session?.confirmation_failure_kind,
  );
  const needsBaselineRecovery = !automaticRetryPending && !validationRetryPending && !waitingEngine
    && (recoveryAction === "RestoreExecutionBaseline" || (!recovery && isRecoverableSession(project)));

  // 恢复入口条（手动 / 自动驾驶共用）
  const recoveryBar = needsBaselineRecovery && (onAcknowledgeRecovery || onRetryCurrent) ? (
    <div className="autopilot-control-bar ap-error">
      <div className="ap-bar-left">
        <span className="ap-bar-status">
          <AlertTriangle size={16} />
          {" "}
          {sessionLost ? "执行中断" : stopFailed ? "暂停失败" : "执行失败"}
        </span>
        {session?.subtask_title && (
          <span className="ap-bar-target">任务：{session.subtask_title}</span>
        )}
        {(session?.failure_message || errorMessage) && (
          <span className="ap-bar-error" title={session?.failure_message || errorMessage}>
            {(session?.failure_message || errorMessage || "").slice(0, 80)}
            {(session?.failure_message || errorMessage || "").length > 80 ? "…" : ""}
          </span>
        )}
        <span className="ap-bar-warning">
          请先恢复执行基线；未完成前不会谎称已恢复到安全状态。
        </span>
      </div>
      <div className="ap-bar-right">
        <button className="ap-bar-btn" disabled={busy} onClick={onSync} title="同步项目状态">
          <RotateCcw size={14} /> 同步
        </button>
        {onAcknowledgeRecovery && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onAcknowledgeRecovery}>
            <RotateCcw size={14} /> 恢复基线并继续
          </button>
        )}
        {!onAcknowledgeRecovery && onRetryCurrent && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onRetryCurrent}>
            <RotateCcw size={14} /> 恢复基线并重试
          </button>
        )}
        {apActive && (
          <button className="ap-bar-btn" disabled={busy} onClick={() => onToggle(false)}>
            <Square size={14} /> 关闭
          </button>
        )}
      </div>
    </div>
  ) : null;

  const confirmationBar = confirmationBlocked ? (
    <div className="autopilot-control-bar ap-error">
      <div className="ap-bar-left">
        <span className="ap-bar-status"><GitBranch size={16} /> Git 确认受阻</span>
        {session?.subtask_title && <span className="ap-bar-target">任务：{session.subtask_title}</span>}
        <span className="ap-bar-warning">代码与质量结果已保留，不需要恢复执行基线。</span>
        {session?.failure_message && (
          <span className="ap-bar-error" title={session.failure_message}>
            {session.failure_message.slice(0, 100)}{session.failure_message.length > 100 ? "…" : ""}
          </span>
        )}
      </div>
      <div className="ap-bar-right">
        <button className="ap-bar-btn" disabled={busy} onClick={onSync} title="同步项目状态">
          <RotateCcw size={14} /> 同步
        </button>
        {onRetryGitConfirmation && confirmationPresentation.canRetry && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onRetryGitConfirmation}>
            <GitBranch size={14} /> 重新确认提交
          </button>
        )}
        <span className="ap-bar-hint">{confirmationPresentation.hint}</span>
      </div>
    </div>
  ) : null;

  const engineRecoveryBar = waitingEngine && !automaticRetryPending ? (
    <div className="autopilot-control-bar ap-error">
      <div className="ap-bar-left">
        <span className="ap-bar-status"><AlertTriangle size={16} /> 执行引擎阻断</span>
        {session?.subtask_title && <span className="ap-bar-target">任务：{session.subtask_title}</span>}
        <span className="ap-bar-error" title={session?.failure_message || errorMessage}>
          {(session?.failure_message || errorMessage || "执行引擎不可用").slice(0, 100)}
        </span>
        <span className="ap-bar-warning">请修复额度或认证，也可以在顶部设置中切换引擎。</span>
      </div>
      <div className="ap-bar-right">
        <button className="ap-bar-btn" disabled={busy} onClick={onSync} title="同步项目状态">
          <RotateCcw size={14} /> 同步
        </button>
        {onAcknowledgeRecovery && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onAcknowledgeRecovery}>
            <RotateCcw size={14} /> 检查引擎并重试
          </button>
        )}
        {apActive && (
          <button className="ap-bar-btn" disabled={busy} onClick={() => onToggle(false)}>
            <Square size={14} /> 关闭
          </button>
        )}
      </div>
    </div>
  ) : null;

  if (confirmationBar) return confirmationBar;
  if (engineRecoveryBar) return engineRecoveryBar;

  if (!apActive) {
    // 未激活：先显示恢复入口，再显示激活入口（托管层活跃时互斥）
    if (recoveryBar) return recoveryBar;
    if (mfActive) {
      const managed = getManagedFlowPresentation(
        mfState!,
        project.workflow_state.current_step,
        project.milestone_draft,
      );
      return (
        <div className="autopilot-control-bar" style={{ background: "#f6f8fa", borderColor: "#d0d7de" }}>
          <span className="ap-bar-status" style={{ color: "#656d76" }}>
            <WandSparkles size={16} /> {managed.statusLabel}，自动驾驶不可用
          </span>
          {mfState?.last_action && (
            <span className="ap-bar-action" title={mfState.last_action}>{mfState.last_action}</span>
          )}
          <span className="ap-bar-target">目标：{managed.targetLabel}</span>
          <span className="ap-bar-target">动作：{managed.actionLabel}</span>
          <span className="ap-bar-target">心跳：{managed.heartbeatLabel}</span>
          {managed.detail && managed.detail !== mfState?.last_action && (
            <span className="ap-bar-error" title={managed.detail}>{managed.detail}</span>
          )}
          <button className="ap-bar-btn" disabled={busy} onClick={onStopManagedFlow} title="停止托管并转为手动处理">
            <Square size={14} /> 停止托管
          </button>
        </div>
      );
    }
    if (project.workflow_state.top_level_phase !== "Console") return null;
    return (
      <div className="autopilot-control-bar">
        <span className="ap-bar-status">
          <Play size={16} /> {canActivate ? "自动驾驶未激活" : "请先完成大阶段批准"}
        </span>
        <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy || !canActivate}
          title={canActivate ? "激活自动驾驶" : "自动驾驶只能从大阶段选择及后续步骤激活"}
          onClick={() => onToggle(true)}>
          <WandSparkles size={14} /> 激活自动驾驶
        </button>
      </div>
    );
  }

  // 自动驾驶激活且存在执行恢复动作：只显示一个主恢复动作
  if (recoveryBar) return recoveryBar;

  const errorActions = getAutopilotErrorActions(
    runStatus ?? "Paused",
    recoveryAction,
  );
  const showGenericResume = errorActions.canResume;
  const showRetryAdvance = errorActions.canRetryAdvance;
  const recoveryStatus = recovery ? getRecoveryStatusLabel(recovery) : "";

  return (
    <div className={`autopilot-control-bar ${runStatus === "Running" || isExecuting ? "ap-running" : ""} ${runStatus === "ErrorStopped" ? "ap-error" : ""}`}>
      {/* 左侧状态 */}
      <div className="ap-bar-left">
        <span className="ap-bar-status">
          {isExecuting ? <Play size={16} className="ap-spin" /> :
           runStatus === "Running" ? <WandSparkles size={16} /> :
           runStatus === "Paused" ? <Pause size={16} /> :
           runStatus === "ErrorStopped" ? <AlertTriangle size={16} /> :
           runStatus === "WaitingMilestoneReview" ? <Square size={16} /> :
           <WandSparkles size={16} />}
          {" "}
          {validationRetryPending ? "等待验证重试" :
           validating ? "验证中" :
           isExecuting ? "执行中" :
           automaticRetryPending ? "等待自动重试" :
           runStatus === "Running" ? "自动推进中" :
           runStatus === "Paused" ? "已暂停" :
           runStatus === "ErrorStopped" ? "出错停止" :
           runStatus === "WaitingMilestoneReview" ? "等待大阶段审阅" :
           "未知"}
        </span>
        {targetLabel && <span className="ap-bar-target">目标：{targetLabel}</span>}
        {currentAction && (
          <span className="ap-bar-action" title={apState?.current_action_kind}>
            <Activity size={13} /> 当前：{currentAction}
          </span>
        )}
        {retryAt && (
          <span className="ap-bar-warning" title={apState?.next_retry_at}>
            <Clock3 size={13} /> 重试 {retryCount}/3 · {retryAt}
          </span>
        )}
        {validationRetryAt && (
          <span className="ap-bar-warning" title={recovery?.next_validation_retry_at}>
            <Clock3 size={13} /> 审查重试 {recovery?.validation_retry_count ?? 0}/{recovery?.max_validation_retries ?? 0} · {validationRetryAt}
          </span>
        )}
        {validationStage && (
          <span className="ap-bar-action" title={validationStage}>
            <ScanSearch size={13} /> 验证阶段：{getVerificationStageLabel(validationStage)}
          </span>
        )}
        {heartbeatAt && (
          <span className="ap-bar-target" title={apState?.heartbeat_at}>
            心跳 {heartbeatAt}
          </span>
        )}
        {heartbeatStale && <span className="ap-bar-error">心跳异常：后台验证可能已停止更新</span>}
        {(recoveryStatus || lastAction) && (
          <span className="ap-bar-action" title={recoveryStatus || lastAction}>
            {recoveryStatus || lastAction}
          </span>
        )}
        {qualityStatuses.map(status => (
          <span
            key={status.key}
            className="ap-bar-warning"
            title={status.label}
            style={{
              color: status.tone === "success" ? "#1a7f37"
                : status.tone === "error" ? "#cf222e"
                  : status.tone === "warning" ? "#9a6700" : "#656d76",
            }}
          >
            {status.key === "automated-test" ? <TestTube2 size={13} />
              : status.key === "code-review" ? <ScanSearch size={13} />
                : status.key === "review-protocol" ? <Activity size={13} />
                  : <FileQuestion size={13} />}
            {" "}{status.label}
          </span>
        ))}
        {errorMessage && runStatus === "ErrorStopped" && (
          <span className="ap-bar-error" title={errorMessage}>{errorMessage.slice(0, 80)}{errorMessage.length > 80 ? "…" : ""}</span>
        )}
        {waitingEngine && !automaticRetryPending && (
          <span className="ap-bar-warning">请在顶部引擎设置中充值、修复认证或切换引擎，再重试当前任务。</span>
        )}
        {mfActive && <span className="ap-bar-mutex">托管层活跃</span>}
      </div>

      {/* 右侧操作：按 recovery_action 只显示一个主恢复动作 */}
      <div className="ap-bar-right">
        <button className="ap-bar-btn" disabled={busy} onClick={onSync} title="同步项目状态">
          <RotateCcw size={14} /> 同步
        </button>

        {/* 真实执行中：In Stop + 完成后暂停 */}
        {isExecuting && !validationRetryPending && (
          <>
            <button className="ap-bar-btn ap-bar-btn-danger" disabled={busy} onClick={onPauseNow}>
              <Square size={14} /> 立即暂停
            </button>
            <button className="ap-bar-btn" disabled={busy} onClick={onPauseAfterCurrent}>
              <Pause size={14} /> 完成后暂停
            </button>
          </>
        )}

        {(validationRetryPending || (!isExecuting && runStatus === "Running" && validating)) && (
          <button className="ap-bar-btn" disabled={busy} onClick={onPauseNow}>
            <Pause size={14} /> 暂停验证
          </button>
        )}

        {/* 规划推进中只提供普通暂停，不触发 Git 回退。 */}
        {!isExecuting && runStatus === "Running" && !validating && !validationRetryPending && (
          <button className="ap-bar-btn" disabled={busy} onClick={onPauseNow}>
            <Pause size={14} /> 暂停自动驾驶
          </button>
        )}

        {/* 暂停或可恢复错误：恢复 + 关闭（执行基线恢复场景已在上方单独处理） */}
        {showGenericResume && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onResume}>
            <Play size={14} /> 恢复
          </button>
        )}

        {showRetryAdvance && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onResume}>
            <Play size={14} /> 重新尝试自动推进
          </button>
        )}

        {errorActions.canRegeneratePlan && onRegeneratePlan && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onRegeneratePlan}>
            <RotateCcw size={14} /> 重新生成计划
          </button>
        )}

        {errorActions.canPrepareWorkspace && onPrepareWorkspace && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onPrepareWorkspace}>
            <GitBranch size={14} /> 准备 Git
          </button>
        )}

        {errorActions.canRefreshWorkspace && onRefreshWorkspace && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onRefreshWorkspace}>
            <RotateCcw size={14} /> 刷新工作区
          </button>
        )}

        {waitingEngine && !automaticRetryPending && onAcknowledgeRecovery && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onAcknowledgeRecovery}>
            <RotateCcw size={14} /> 检查引擎并重试
          </button>
        )}

        {validationHumanBoundary && onResolveHumanRecovery && (
          <>
            {reviewServiceBlocked && (
              <button
                className="ap-bar-btn ap-bar-btn-primary"
                disabled={busy}
                onClick={() => window.dispatchEvent(new Event("metheus:open-decision-settings"))}
              >
                <Settings2 size={14} /> 打开决策模型设置
              </button>
            )}
            <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy}
              onClick={() => onResolveHumanRecovery("revalidate")}>
              <RotateCcw size={14} /> 重新验证
            </button>
          </>
        )}

        {recovery?.phase === "WaitingHuman" && !validationHumanBoundary && !automaticRetryPending && onResolveHumanRecovery && (
          <>
            <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy}
              onClick={() => onResolveHumanRecovery("retest")}>
              <RotateCcw size={14} /> 手动修复后复测
            </button>
            <button className="ap-bar-btn" disabled={busy}
              onClick={() => onResolveHumanRecovery("restore_and_retry")}>
              <GitBranch size={14} /> 恢复基线并重试
            </button>
            {!recovery.replan_attempted && (
              <button className="ap-bar-btn" disabled={busy}
                onClick={() => onResolveHumanRecovery("regenerate_plan")}>
                <RotateCcw size={14} /> 重新规划当前任务
              </button>
            )}
            {recovery.error_kind !== "ExecutionError" && recovery.error_kind !== "EngineBlocked" && (
              <>
                <button className="ap-bar-btn" disabled={busy}
                  onClick={() => onResolveHumanRecovery("confirm_actual_pass")}>
                  <CheckCircle size={14} /> 确认实际通过
                </button>
                <button className="ap-bar-btn" disabled={busy}
                  onClick={() => onResolveHumanRecovery("accept_deviation")}>
                  <AlertTriangle size={14} /> 接受偏差并继续
                </button>
              </>
            )}
            <button className="ap-bar-btn" disabled={busy}
              onClick={() => onResolveHumanRecovery("skip_task")}>
              <CircleSlash2 size={14} /> 跳过当前任务
            </button>
          </>
        )}

        {errorActions.canClose && !validationRetryPending && (
          <button className="ap-bar-btn" disabled={busy} onClick={() => onToggle(false)}>
            <Square size={14} /> 关闭
          </button>
        )}

        {/* 大阶段审阅 / 等待人工：只能提示，不显示无效重试 */}
        {(runStatus === "WaitingMilestoneReview" || (recoveryAction === "WaitHumanDecision" && !recovery)) && !showGenericResume && !showRetryAdvance && (
          <span className="ap-bar-hint">请完成人工决策</span>
        )}
      </div>
    </div>
  );
}
