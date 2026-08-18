/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RuntimeOutcomePresentation } from "../runtimeOutcomePresentation";
import { RecoveryResultBanner, RECOVERY_RESULT_DISPLAY_MS } from "./RecoveryResultBanner";

describe("RecoveryResultBanner", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  it("shows the backend result and dismisses it after the bounded display window", () => {
    const onDismiss = vi.fn();
    act(() => root.render(
      <RecoveryResultBanner
        result={{
          title: "执行基线已恢复",
          message: "后台作业已重新接续。",
          baseline: "abc123",
          baseline_summary: "恢复到提交：abc123",
          discarded_files: ["src/a.ts"],
          discarded_files_summary: "已丢弃 1 个文件的执行期改动",
          background_job_started: true,
          background_job_summary: "后台作业：已重新启动",
          next_step: "后台将重新执行当前任务。",
          next_step_summary: "下一步：后台将重新执行当前任务。",
        }}
        onDismiss={onDismiss}
      />,
    ));

    expect(host.textContent).toContain("执行基线已恢复");
    expect(host.textContent).toContain("abc123");
    expect(host.textContent).toContain("已丢弃 1 个文件");
    expect(host.textContent).toContain("src/a.ts");
    expect(host.textContent).toContain("后台作业：已重新启动");
    expect(host.textContent).toContain("下一步：后台将重新执行当前任务");
    act(() => vi.advanceTimersByTime(RECOVERY_RESULT_DISPLAY_MS));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("keeps recovery completion separate from a task still awaiting validation", () => {
    act(() => root.render(
      <RecoveryResultBanner
        result={{
          title: "执行基线已恢复",
          message: "恢复动作已收口。",
          baseline: null,
          baseline_summary: "",
          discarded_files: [],
          discarded_files_summary: "",
          background_job_started: false,
          background_job_summary: "",
          next_step: "validate",
          next_step_summary: "下一步：等待验证结果。",
        }}
        runtimeOutcome={{
          state: "validating",
          statusLabel: "验证中",
          summary: "等待质量和验收事实",
          tone: "active",
          execution: "passed",
          quality: "pending",
          acceptance: "pending",
          confirmation: "required",
          recoveryKind: "AutomaticRecovery",
          syncStatus: "synced",
          syncFresh: true,
          writeAllowed: true,
          writeBlockedReason: "",
        } satisfies RuntimeOutcomePresentation}
        onDismiss={vi.fn()}
      />,
    ));

    expect(host.textContent).toContain("当前任务：验证中");
    expect(host.textContent).toContain("等待质量和验收事实");
    expect(host.textContent).not.toContain("任务已完成");
  });
});
