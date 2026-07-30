/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import TaskConsole from "./TaskConsole";
import type { ExecutionHistoryEntry } from "./types";

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
});
