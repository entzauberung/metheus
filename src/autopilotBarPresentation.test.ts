import { describe, expect, it } from "vitest";
import {
  partitionAutopilotActions,
  resolveAutopilotRuntimePresentation,
  resolveManagedActionSlots,
} from "./autopilotBarPresentation";

describe("autopilot bar presentation", () => {
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
