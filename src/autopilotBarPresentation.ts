import type {
  AutopilotRunStatus,
  RecoveryPresentation,
  RecoveryProgressStatus,
} from "./types";

export type AutopilotBarState = "Running" | "Paused" | "Waiting" | "Recovery" | "Error";

export type AutopilotActionId =
  | "activate"
  | "pause-now"
  | "pause-after-current"
  | "resume"
  | "close"
  | "pause-managed"
  | "resume-managed"
  | "stop-managed";

export interface AutopilotActionSlots<T = AutopilotActionId> {
  primary: T | null;
  secondary: T | null;
  overflow: T[];
}

export interface AutopilotRuntimePresentation {
  state: AutopilotBarState;
  statusLabel: string;
  summary: string;
  actions: AutopilotActionSlots;
}

export type RecoveryBarProgressStatus = RecoveryProgressStatus | "unknown";

export interface RecoveryBarProgressPresentation {
  status: RecoveryBarProgressStatus;
  statusLabel: string;
  summary: string;
  tone: "neutral" | "active" | "warning" | "error";
}

export function resolveRecoveryBarProgress(
  recovery: RecoveryPresentation,
): RecoveryBarProgressPresentation {
  switch (recovery.progress_status) {
    case "inactive":
      return {
        status: "inactive",
        statusLabel: "恢复未运行",
        summary: "当前没有恢复动作",
        tone: "neutral",
      };
    case "queued":
      return {
        status: "queued",
        statusLabel: "自动恢复已排队",
        summary: "等待恢复 worker 领取",
        tone: "active",
      };
    case "scheduled":
      return {
        status: "scheduled",
        statusLabel: "恢复重试已安排",
        summary: "等待计划重试",
        tone: "active",
      };
    case "running":
      return {
        status: "running",
        statusLabel: "自动恢复执行中",
        summary: "恢复动作正在执行",
        tone: "active",
      };
    case "warning":
      return {
        status: "warning",
        statusLabel: "恢复进展延迟",
        summary: "worker 存活，但业务进展延迟",
        tone: "warning",
      };
    case "stalled":
      return {
        status: "stalled",
        statusLabel: "恢复已停滞",
        summary: "等待后端有界超时收口",
        tone: "error",
      };
    case "waiting_human":
      return {
        status: "waiting_human",
        statusLabel: "自动恢复已停止",
        summary: "等待人工处理",
        tone: "error",
      };
    default:
      return {
        status: "unknown",
        statusLabel: "恢复进度未记录",
        summary: "进度未记录",
        tone: "neutral",
      };
  }
}

export function resolveAutopilotBarState(
  runStatus: AutopilotRunStatus | undefined,
  isExecuting: boolean,
): AutopilotBarState {
  if (isExecuting || runStatus === "Running") return "Running";
  if (runStatus === "Paused") return "Paused";
  if (runStatus === "ErrorStopped") return "Error";
  return "Waiting";
}

export function compactAutopilotSummary(
  parts: Array<string | null | undefined>,
  fallback: string,
): string {
  const summary = parts.map(part => part?.trim()).filter(Boolean).join(" · ");
  return summary || fallback;
}

export function resolveAutopilotRuntimePresentation(
  runStatus: AutopilotRunStatus | undefined,
  isExecuting: boolean,
  targetLabel: string,
): AutopilotRuntimePresentation {
  const state = resolveAutopilotBarState(runStatus, isExecuting);
  const statusLabel = isExecuting
    ? "执行中"
    : runStatus === "Running"
      ? "自动推进中"
      : runStatus === "Paused"
        ? "已暂停"
        : runStatus === "ErrorStopped"
          ? "执行错误"
          : "等待人工处理";
  const summary = targetLabel ? `目标：${targetLabel}` : "等待后端分派下一动作";

  if (isExecuting) {
    return {
      state,
      statusLabel,
      summary,
      actions: {
        primary: "pause-now",
        secondary: "pause-after-current",
        overflow: runStatus === "Running" ? [] : ["close"],
      },
    };
  }
  if (runStatus === "Running") {
    return {
      state,
      statusLabel,
      summary,
      actions: { primary: "pause-now", secondary: null, overflow: [] },
    };
  }
  if (runStatus === "Paused") {
    return {
      state,
      statusLabel,
      summary,
      actions: { primary: "resume", secondary: "close", overflow: [] },
    };
  }
  return {
    state,
    statusLabel,
    summary,
    actions: { primary: null, secondary: "close", overflow: [] },
  };
}

export function resolveManagedActionSlots(
  canPause: boolean,
  canResume: boolean,
): AutopilotActionSlots {
  return {
    primary: canPause ? "pause-managed" : canResume ? "resume-managed" : null,
    secondary: "stop-managed",
    overflow: [],
  };
}

export function partitionAutopilotActions<T>(
  primary: T | null,
  secondaryActions: T[],
): AutopilotActionSlots<T> {
  return {
    primary,
    secondary: secondaryActions[0] ?? null,
    overflow: secondaryActions.slice(1),
  };
}
