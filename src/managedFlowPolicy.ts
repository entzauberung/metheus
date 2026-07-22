import type { ManagedFlowState, MilestoneDraft, WorkflowStep } from "./types";

export interface ManagedFlowPresentation {
  statusLabel: string;
  canPause: boolean;
  canResume: boolean;
  resumeLabel: string;
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
