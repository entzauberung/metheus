import type { ManagedFlowState, MilestoneDraft, WorkflowStep } from "./types";

export interface ManagedFlowPresentation {
  statusLabel: string;
  canPause: boolean;
  canResume: boolean;
  resumeLabel: string;
  actionLabel: string;
  targetLabel: string;
  heartbeatLabel: string;
  detail: string;
}

const MANAGED_ACTION_LABELS: Record<string, string> = {
  generate_version_plan: "生成项目方案",
  approve_version_plan: "批准项目方案",
  enter_console: "进入控制台",
  generate_milestone_draft: "生成大阶段草稿",
  check_milestone_draft: "检查大阶段草稿",
  approve_milestone_draft: "批准大阶段草稿",
};

const MANAGED_TARGET_LABELS: Record<string, string> = {
  MilestoneSelection: "完成首个大阶段批准",
};

function formatHeartbeat(value: string): string {
  if (!value) return "尚无心跳";
  const time = new Date(value);
  if (Number.isNaN(time.getTime())) return value;
  return time.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function getManagedFlowPresentation(
  managed: ManagedFlowState,
  step: WorkflowStep,
  draft?: MilestoneDraft,
): ManagedFlowPresentation {
  const canResume = managed.run_status === "Paused" || managed.run_status === "WaitingHuman";
  const resumesIntoApproval = canResume
    && step === "MilestoneApproval"
    && draft?.status === "CheckPassed";
  const statusLabels: Record<ManagedFlowState["run_status"], string> = {
    Running: "托管层运行中",
    Paused: "托管层已暂停",
    WaitingHuman: "托管层等待人工处理",
    ErrorStopped: "托管层因错误停止",
  };

  return {
    statusLabel: statusLabels[managed.run_status],
    canPause: managed.run_status === "Running",
    canResume,
    resumeLabel: resumesIntoApproval ? "继续托管并批准" : "恢复托管",
    actionLabel: managed.current_action
      ? (MANAGED_ACTION_LABELS[managed.current_action] ?? managed.current_action)
      : (managed.last_completed_action
        ? `已完成：${MANAGED_ACTION_LABELS[managed.last_completed_action] ?? managed.last_completed_action}`
        : "等待后端派发动作"),
    targetLabel: MANAGED_TARGET_LABELS[managed.managed_target] ?? managed.managed_target,
    heartbeatLabel: formatHeartbeat(managed.heartbeat_at),
    detail: managed.run_status === "ErrorStopped"
      ? (managed.error_message || managed.last_action)
      : managed.last_action,
  };
}

export interface MilestoneApprovalPolicy {
  canApprove: boolean;
  description: string;
  statusLabel: string;
}

export function getMilestoneApprovalPolicy(draft?: MilestoneDraft): MilestoneApprovalPolicy {
  const canApprove = draft?.status === "CheckPassed"
    && Boolean(draft.check_result?.trim())
    && draft.candidate_milestones.length > 0;
  return canApprove
    ? { canApprove: true, description: "质量检查已通过", statusLabel: "待批准" }
    : { canApprove: false, description: "大阶段状态需要同步", statusLabel: "状态异常" };
}
