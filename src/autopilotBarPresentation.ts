import type { AutopilotRunStatus } from "./types";

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
