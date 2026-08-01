/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectSyncStatus } from "../hooks/useProjectStateSync";
import { createProjectSyncState } from "../test/projectSyncStateFactory";
import type { TaskControlDetailStatus } from "../types";
import { SyncStatusIndicator } from "./SyncStatusIndicator";

function state(status: ProjectSyncStatus) {
  return createProjectSyncState({
    status,
    subscriptionStatus: status === "idle" ? "idle" : "connected",
    lastSuccessfulSyncAt: status === "idle" ? null : "2026-07-31T05:00:00Z",
    consecutiveFailures: status === "delayed" ? 1 : status === "disconnected" ? 3 : 0,
    lastEventSequence: 4,
    pendingRevision: null,
    lastError: "",
    taskControlEventSequence: 0,
    taskControlProcessStartId: "",
    taskControlProjectRevision: 0,
    taskControlTreeRevision: 0,
    taskControlDirty: false,
    taskControlSnapshotVersion: "",
    taskControlActionId: null,
    taskControlMode: null,
  });
}

describe("SyncStatusIndicator", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it.each([
    ["synced", "已同步"],
    ["syncing", "正在同步"],
    ["delayed", "同步延迟"],
    ["disconnected", "后端断开"],
  ] as const)("shows %s health explicitly", (status, label) => {
    act(() => root.render(<SyncStatusIndicator state={state(status)} onRetry={vi.fn()} />));
    expect(host.querySelector(`[data-sync-status='${status}']`)?.textContent).toContain(label);
    expect(host.querySelector("[role='status']")?.getAttribute("aria-live")).toBe("polite");
  });

  it("offers an explicit retry for unhealthy synchronization", () => {
    const onRetry = vi.fn();
    act(() => root.render(<SyncStatusIndicator state={state("disconnected")} onRetry={onRetry} />));
    const retry = host.querySelector("button");
    act(() => retry?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("makes terminal reconciliation and delayed retry visible", () => {
    act(() => root.render(
      <SyncStatusIndicator
        state={state("synced")}
        terminalPhase="terminal_reconciling"
        onRetry={vi.fn()}
      />,
    ));
    expect(host.textContent).toContain("正在获取最终状态");
    expect(host.textContent).toContain("后台动作已结束");

    act(() => root.render(
      <SyncStatusIndicator
        state={state("synced")}
        terminalPhase="terminal_delayed"
        onRetry={vi.fn()}
      />,
    ));
    expect(host.textContent).toContain("最终状态延迟");
  });

  it("exposes a reconnecting event channel even when snapshots still work", () => {
    const reconnecting = state("synced");
    reconnecting.subscriptionStatus = "reconnecting";
    act(() => root.render(<SyncStatusIndicator state={reconnecting} onRetry={vi.fn()} />));

    const indicator = host.querySelector("[data-subscription-status='reconnecting']");
    expect(indicator?.getAttribute("data-sync-status")).toBe("delayed");
    expect(indicator?.textContent).toContain("通知重连中");
    expect(indicator?.getAttribute("title")).toContain("低频快照兜底");
  });

  it.each([
    ["idle", "任务详情未请求"],
    ["syncing", "任务详情同步中"],
    ["ready", "任务详情已同步"],
    ["unavailable", "任务详情暂不可用"],
  ] as const)("shows task-control detail status %s", (detailStatus, label) => {
    const syncState = state("synced");
    syncState.taskControlDetailStatus = detailStatus as TaskControlDetailStatus;
    act(() => root.render(<SyncStatusIndicator state={syncState} onRetry={vi.fn()} />));
    const indicator = host.querySelector("[role='status']");
    expect(indicator?.getAttribute("data-task-control-detail-status")).toBe(detailStatus);
    expect(host.querySelector("[data-testid='task-control-detail-status']")?.textContent)
      .toContain(label);
    if (detailStatus === "unavailable") {
      expect(indicator?.getAttribute("data-sync-status")).toBe("synced");
      expect(indicator?.textContent).not.toContain("后端断开");
    }
  });
});
