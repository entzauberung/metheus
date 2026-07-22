// src/components/AutopilotControlBar.tsx — 全局自动驾驶控制条
// 在所有 Console 页面顶部显示自动驾驶状态与操作入口
import { Pause, Play, RotateCcw, Square, WandSparkles, AlertTriangle, GitBranch, CheckCircle } from "lucide-react";
import type { Project, PipelineState, AutopilotRecoveryAction } from "../types";
import { getAutopilotErrorActions } from "../autopilotPolicy";
import { getManagedFlowPresentation } from "../managedFlowPolicy";

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
  onResolveHumanRecovery?: (resolution: "retest" | "restore_and_retry" | "regenerate_plan" | "human_override") => Promise<void>;
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

export function AutopilotControlBar({
  project, executionStatus, busy,
  onToggle, onStopManagedFlow, onPauseNow, onPauseAfterCurrent, onResume, onSync,
  onRetryCurrent, onAcknowledgeRecovery, onRegeneratePlan, onPrepareWorkspace,
  onRefreshWorkspace, onResolveHumanRecovery,
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
  const errorMessage = apState?.error_message;
  const recoveryAction: AutopilotRecoveryAction = apState?.recovery_action ?? "None";
  const recovery = project.workflow_state.recovery_state;
  const targetMs = project.milestones.find(m => m.id === project.workflow_state.autopilot_target_milestone_id);
  const targetLabel = targetMs?.title ?? project.workflow_state.autopilot_target_milestone_id;

  // 先判断失败会话，再判断自动驾驶是否激活 — 手动模式也要看到恢复入口
  const session = project.execution_session;
  const sessionKey = sessionStatusKey(session?.status);
  const sessionLost = sessionKey === "session_lost";
  const stopFailed = sessionKey === "stop_failed";
  const needsBaselineRecovery =
    recoveryAction === "RestoreExecutionBaseline" || (!recovery && isRecoverableSession(project));

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
  const lastAttempt = recovery?.attempt_history?.length
    ? recovery.attempt_history[recovery.attempt_history.length - 1]
    : undefined;
  const recoveryStatus = recovery ? {
    Diagnosing: recovery.replan_attempted && recovery.attempt === 0
      ? "重规划完成，准备从基线重新执行"
      : recovery.active_issues?.length
      ? `正在分析 ${recovery.active_issues.length} 个未满足验收项`
      : "正在诊断错误",
    Repairing: recovery.replan_attempted
      ? "正在执行重规划后的当前任务"
      : `正在执行第 ${recovery.attempt}/${recovery.max_attempts} 次修复`,
    Retesting: lastAttempt
      ? `正在重新测试；上一轮解决 ${lastAttempt.resolved_issue_ids.length} 项，剩余 ${lastAttempt.remaining_issue_ids.length + lastAttempt.regressed_issue_ids.length} 项`
      : "正在重新测试",
    Replanning: "常规修复耗尽，正在重新规划当前任务",
    Recovered: "自动修复成功，继续执行",
    WaitingHuman: recovery.error_kind === "TestUnavailable" && recovery.attempt === 0
      ? "自动修复未启动：测试或审查不可用"
      : "自动恢复已耗尽，等待人工处理",
  }[recovery.phase] : "";

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
          {isExecuting ? "执行中" :
           runStatus === "Running" ? "自动推进中" :
           runStatus === "Paused" ? "已暂停" :
           runStatus === "ErrorStopped" ? "出错停止" :
           runStatus === "WaitingMilestoneReview" ? "等待大阶段审阅" :
           "未知"}
        </span>
        {targetLabel && <span className="ap-bar-target">目标：{targetLabel}</span>}
        {(recoveryStatus || lastAction) && (
          <span className="ap-bar-action" title={recoveryStatus || lastAction}>
            {recoveryStatus || lastAction}
          </span>
        )}
        {errorMessage && runStatus === "ErrorStopped" && (
          <span className="ap-bar-error" title={errorMessage}>{errorMessage.slice(0, 80)}{errorMessage.length > 80 ? "…" : ""}</span>
        )}
        {mfActive && <span className="ap-bar-mutex">托管层活跃</span>}
      </div>

      {/* 右侧操作：按 recovery_action 只显示一个主恢复动作 */}
      <div className="ap-bar-right">
        <button className="ap-bar-btn" disabled={busy} onClick={onSync} title="同步项目状态">
          <RotateCcw size={14} /> 同步
        </button>

        {/* 真实执行中：In Stop + 完成后暂停 */}
        {isExecuting && (
          <>
            <button className="ap-bar-btn ap-bar-btn-danger" disabled={busy} onClick={onPauseNow}>
              <Square size={14} /> 立即暂停
            </button>
            <button className="ap-bar-btn" disabled={busy} onClick={onPauseAfterCurrent}>
              <Pause size={14} /> 完成后暂停
            </button>
          </>
        )}

        {/* 规划推进中只提供普通暂停，不触发 Git 回退。 */}
        {!isExecuting && runStatus === "Running" && (
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

        {recovery?.phase === "WaitingHuman" && onResolveHumanRecovery && (
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
            <button className="ap-bar-btn" disabled={busy}
              onClick={() => onResolveHumanRecovery("human_override")}>
              <CheckCircle size={14} /> 人工核验通过
            </button>
          </>
        )}

        {errorActions.canClose && (
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
