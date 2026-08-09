/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import TaskConsole from "./TaskConsole";
import type { ExecutionHistoryEntry, RecoveryPresentation } from "./types";

describe("TaskConsole task navigation", () => {
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
    vi.restoreAllMocks();
  });

  it("opens the shared inspector selection without leaving or scrolling the log tab", () => {
    const onOpenTask = vi.fn();
    const history: ExecutionHistoryEntry[] = [{
      timestamp: "2026-07-29T00:00:00Z",
      level: "info",
      event_type: "UserExecute",
      source: "User",
      text: "打开深层任务",
      subtask_id: "deep-task",
    }];
    act(() => {
      root.render(
        <TaskConsole
          projectPath="/tmp/project"
          projectName="project"
          executionStatus={null}
          testLogs={[]}
          executionHistory={history}
          selectedTaskId="deep-task"
          onOpenTask={onOpenTask}
        />,
      );
    });

    const logList = host.querySelector(".execution-log-list") as HTMLDivElement;
    logList.scrollTop = 36;
    const linkedEntry = host.querySelector(".execution-log-entry.has-task-link");
    act(() => linkedEntry?.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(onOpenTask).toHaveBeenCalledWith("deep-task");
    expect(logList.scrollTop).toBe(36);
    expect(host.querySelector('[role="tab"][data-state="active"]')?.textContent).toContain("执行日志");
    expect(host.textContent).not.toContain("任务控制");
    expect(linkedEntry?.getAttribute("aria-current")).toBe("true");
  });

  it("uses the backend recovery presentation instead of local validation fields", () => {
    const recovery = {
      kind: "ValidationRetry",
      validation_phase_label: "请求代码审查",
      validation_retry_count: 2,
      validation_retry_limit: 3,
      next_validation_retry_at: "2026-08-01T00:00:05Z",
    } as RecoveryPresentation;
    act(() => {
      root.render(
        <TaskConsole
          projectPath="/tmp/project"
          executionStatus={null}
          testLogs={[]}
          verificationStage="Completed"
          validationRetryCount={0}
          validationRetryLimit={1}
          recoveryPresentation={recovery}
        />,
      );
    });

    expect(host.textContent).toContain("验证阶段：请求代码审查");
    expect(host.textContent).toContain("审查重试：2/3");
    expect(host.textContent).not.toContain("验证阶段：验证完成");
  });

  it("hides thought and debug output by default and reveals it only through the debug filter", () => {
    const history: ExecutionHistoryEntry[] = [
      {
        timestamp: "2026-08-07T00:00:00Z",
        level: "debug",
        event_type: "SystemAdvance",
        source: "System",
        text: "private chain thought",
      },
      {
        timestamp: "2026-08-07T00:00:01Z",
        level: "info",
        event_type: "ExecutorComplete",
        source: "System",
        text: "final answer preserved",
      },
    ];
    act(() => {
      root.render(
        <TaskConsole
          projectPath="/tmp/project"
          executionStatus={{
            status: "Running",
            current_log: '{"type":"thought","text":"live hidden thought"}',
            log_history: [],
          } as never}
          testLogs={[]}
          executionHistory={history}
        />,
      );
    });

    expect(host.textContent).toContain("final answer preserved");
    expect(host.textContent).not.toContain("private chain thought");
    expect(host.textContent).not.toContain("live hidden thought");
    const debugFilter = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((candidate) => candidate.textContent === "调试");
    if (!debugFilter) throw new Error("缺少调试日志筛选");
    expect(debugFilter.getAttribute("aria-pressed")).toBe("false");
    act(() => debugFilter.click());
    expect(debugFilter.getAttribute("aria-pressed")).toBe("true");
    expect(host.textContent).toContain("private chain thought");
    expect(host.textContent).toContain("live hidden thought");
    expect(host.textContent).not.toContain('{"type":"thought"');
  });

  it("does not render current_log again when it is already in runtime history", () => {
    act(() => {
      root.render(
        <TaskConsole
          projectPath="/tmp/project"
          executionStatus={{
            status: "Running",
            current_log: "[stdout] same result",
            log_history: [{
              timestamp: "2026-08-08T00:00:00Z",
              level: "info",
              text: "[stdout] same result",
              source: "stdout",
              correlation_id: "result-1",
            }],
          } as never}
          testLogs={[]}
        />,
      );
    });

    expect([...host.querySelectorAll(".execution-log-text")]
      .filter((entry) => entry.textContent === "[stdout] same result")).toHaveLength(1);
    expect(host.querySelector(".log-live")).toBeNull();
  });

  it("shows testLogs on the unified timeline with an error level and source", () => {
    act(() => {
      root.render(
        <TaskConsole
          projectPath="/tmp/project"
          executionStatus={null}
          testLogs={[{
            subtask_title: "定向验收",
            status: "rejected",
            reason: "断言失败",
          }]}
        />,
      );
    });

    const entry = host.querySelector(".execution-log-entry.log-error");
    expect(entry?.textContent).toContain("定向验收：断言失败");
    expect(entry?.textContent).toContain("test");
  });
});
