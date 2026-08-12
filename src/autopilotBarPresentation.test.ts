import { describe, expect, it } from "vitest";
import {
  partitionAutopilotActions,
  resolveRecoveryBarProgress,
  resolveAutopilotRuntimePresentation,
  resolveManagedActionSlots,
} from "./autopilotBarPresentation";

describe("autopilot bar presentation", () => {
  it.each([
    ["inactive", "恢复未运行", "当前没有恢复动作", "neutral"],
    ["queued", "自动恢复已排队", "等待恢复 worker 领取", "active"],
    ["scheduled", "恢复重试已安排", "等待计划重试", "active"],
    ["running", "自动恢复执行中", "恢复动作正在执行", "active"],
    ["warning", "恢复进展延迟", "worker 存活，但业务进展延迟", "warning"],
    ["stalled", "恢复已停滞", "等待后端有界超时收口", "error"],
    ["waiting_human", "自动恢复已停止", "等待人工处理", "error"],
  ] as const)("maps %s recovery progress without parsing title or reason", (
    status,
    statusLabel,
    summary,
    tone,
  ) => {
    const view = resolveRecoveryBarProgress({
      progress_status: status,
      title: "不可用于推断",
      reason: "不可用于推断",
    } as never);
    expect(view).toEqual({ status, statusLabel, summary, tone });
  });

  it("uses a neutral compatibility state for an old recovery DTO", () => {
    expect(resolveRecoveryBarProgress({ title: "自动恢复中" } as never)).toEqual({
      status: "unknown",
      statusLabel: "恢复进度未记录",
      summary: "进度未记录",
      tone: "neutral",
    });
  });

  it("keeps Running in one primary slot with a low-volatility summary", () => {
    const view = resolveAutopilotRuntimePresentation("Running", false, "交付设置体验");
    expect(view).toEqual({
      state: "Running",
      statusLabel: "自动推进中",
      summary: "目标：交付设置体验",
      actions: { primary: "pause-now", secondary: null, overflow: [] },
    });
  });

  it("maps Executing pause choices to stable primary and secondary slots", () => {
    const view = resolveAutopilotRuntimePresentation("Running", true, "执行任务");
    expect(view.state).toBe("Running");
    expect(view.statusLabel).toBe("执行中");
    expect(view.actions).toEqual({
      primary: "pause-now",
      secondary: "pause-after-current",
      overflow: [],
    });
  });

  it("maps Paused and Error without inventing backend actions", () => {
    expect(resolveAutopilotRuntimePresentation("Paused", false, "").actions)
      .toEqual({ primary: "resume", secondary: "close", overflow: [] });
    expect(resolveAutopilotRuntimePresentation("ErrorStopped", false, "").actions)
      .toEqual({ primary: null, secondary: "close", overflow: [] });
  });

  it("maps Managed pause or resume into the same primary slot", () => {
    expect(resolveManagedActionSlots(true, false))
      .toEqual({ primary: "pause-managed", secondary: "stop-managed", overflow: [] });
    expect(resolveManagedActionSlots(false, true))
      .toEqual({ primary: "resume-managed", secondary: "stop-managed", overflow: [] });
    expect(resolveManagedActionSlots(false, false))
      .toEqual({ primary: null, secondary: "stop-managed", overflow: [] });
  });

  it("keeps every recovery action while bounding visible slots", () => {
    expect(partitionAutopilotActions("recover", ["sync", "retry", "close"]))
      .toEqual({
        primary: "recover",
        secondary: "sync",
        overflow: ["retry", "close"],
      });
  });
});
