// src/components/AutopilotControlBar.tsx — 全局自动驾驶控制条
// 在所有 Console 页面顶部显示自动驾驶状态与操作入口
import { Pause, Play, RotateCcw, Square, WandSparkles, AlertTriangle } from "lucide-react";
import type { Project, PipelineState } from "../types";

export interface AutopilotControlBarProps {
  project: Project;
  executionStatus?: PipelineState | null;
  busy: boolean;
  onToggle: (active: boolean) => Promise<void>;
  onPauseNow: () => Promise<void>;
  onPauseAfterCurrent: () => Promise<void>;
  onResume: () => Promise<void>;
  onSync: () => Promise<void>;
  onRetryCurrent?: () => Promise<void>;
  onAcknowledgeRecovery?: () => Promise<void>;
}

export function AutopilotControlBar({
  project, executionStatus, busy,
  onToggle, onPauseNow, onPauseAfterCurrent, onResume, onSync,
  onRetryCurrent, onAcknowledgeRecovery,
}: AutopilotControlBarProps) {
  const apActive = project.workflow_state.autopilot_active === true;
  const apState = project.workflow_state.autopilot_state;
  const mfActive = project.workflow_state.managed_flow_state?.active === true;
  const isExecuting = executionStatus?.status === "Running";
  const activationSteps = new Set([
    "MilestoneSelection", "MidStageGeneration", "MidStageCheck", "MidStageApproval",
    "MidStageSelection", "PlanGeneration", "PlanCheck", "PlanApproving", "Execution",
  ]);
  const canActivate = activationSteps.has(project.workflow_state.current_step);
  const currentMilestone = project.milestones.find(m => m.id === project.current_milestone_id);
  const currentMidStage = currentMilestone?.mid_stages.find(m => m.id === project.current_mid_stage_id);
  const canRetryCurrent = currentMidStage?.subtasks.some(subtask =>
    subtask.status === "Rejected"
      || subtask.status === "AwaitingConfirmation"
      || (subtask.status === "Pending" && (subtask.retry_count ?? 0) > 0)
  ) === true;

  const runStatus = apState?.run_status;
  const lastAction = apState?.last_action;
  const errorMessage = apState?.error_message;
  const targetMs = project.milestones.find(m => m.id === project.workflow_state.autopilot_target_milestone_id);
  const targetLabel = targetMs?.title ?? project.workflow_state.autopilot_target_milestone_id;

  // 检查启动恢复状态
  const sessionLost = project.execution_session?.status === "session_lost";

  if (!apActive) {
    // 未激活：显示激活入口（托管层活跃时互斥）
    if (mfActive) {
      return (
        <div className="autopilot-control-bar" style={{ background: "#f6f8fa", borderColor: "#d0d7de" }}>
          <span className="ap-bar-status" style={{ color: "#656d76" }}>
            <WandSparkles size={16} /> 托管层运行中，自动驾驶不可用
          </span>
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
        {lastAction && <span className="ap-bar-action" title={lastAction}>{lastAction}</span>}
        {errorMessage && runStatus === "ErrorStopped" && (
          <span className="ap-bar-error" title={errorMessage}>{errorMessage.slice(0, 80)}{errorMessage.length > 80 ? "…" : ""}</span>
        )}
        {sessionLost && (
          <span className="ap-bar-warning">检测到执行中断，任务已恢复到待执行状态。</span>
        )}
        {mfActive && <span className="ap-bar-mutex">托管层活跃</span>}
      </div>

      {/* 右侧操作 */}
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

        {/* 暂停或可恢复错误：恢复 + 关闭 */}
        {(runStatus === "Paused" || (runStatus === "ErrorStopped" && !sessionLost)) && (
          <>
            <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onResume}>
              <Play size={14} /> 恢复
            </button>
            <button className="ap-bar-btn" disabled={busy} onClick={() => onToggle(false)}>
              <Square size={14} /> 关闭
            </button>
          </>
        )}

        {/* 大阶段审阅：只能提示 */}
        {runStatus === "WaitingMilestoneReview" && (
          <span className="ap-bar-hint">请完成 A/B/C 决策</span>
        )}

        {/* 执行中断恢复确认 */}
        {sessionLost && onAcknowledgeRecovery && (
          <button className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={onAcknowledgeRecovery}>
            确认中断
          </button>
        )}

        {/* 错误恢复操作 */}
        {runStatus === "ErrorStopped" && canRetryCurrent && onRetryCurrent && (
          <button className="ap-bar-btn" disabled={busy} onClick={onRetryCurrent}>
            <RotateCcw size={14} /> 重试当前任务
          </button>
        )}
      </div>
    </div>
  );
}
