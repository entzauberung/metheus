/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, Subtask } from "./types";
import ExecutionTree from "./ExecutionTree";

function task(id: string, children: Subtask[] = [], autoTag?: string): Subtask {
  return {
    id,
    title: `任务 ${id}`,
    status: children.length > 0 ? "Pending" : "AwaitingConfirmation",
    child_tasks: children,
    auto_tag: autoTag,
  } as Subtask;
}

function treeProject(currentTaskId = ""): Project {
  const level4 = task("level-4", [], "v1.2.3-task");
  const level3 = task("level-3", [level4]);
  const level2 = task("level-2", [level3]);
  const level1 = task("level-1", [level2]);
  return {
    current_milestone_id: "milestone-1",
    current_mid_stage_id: "mid-stage-1",
    workflow_state: {
      current_step: "Execution",
    },
    milestones: [{
      id: "milestone-1",
      title: "大阶段",
      version: "v1",
      status: "InProgress",
      subtasks: [],
      mid_stages: [{
        id: "mid-stage-1",
        title: "中阶段",
        version: "v1.1",
        status: "InProgress",
        subtasks: [level1],
      }],
    }],
    execution_session: currentTaskId ? { subtask_id: currentTaskId } : undefined,
  } as unknown as Project;
}

describe("ExecutionTree recursive subtasks", () => {
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

  function render(project: Project, onOpenTask = vi.fn()) {
    act(() => {
      root.render(
        <ExecutionTree
          project={project}
          projectPath="/tmp/project"
          onSelectMilestone={vi.fn(async () => undefined)}
          onSelectMidStage={vi.fn(async () => undefined)}
          onOpenTask={onOpenTask}
        />,
      );
    });
  }

  it("renders four levels, their statuses, and the leaf auto tag", () => {
    render(treeProject("level-4"));

    for (const id of ["level-1", "level-2", "level-3", "level-4"]) {
      expect(host.textContent).toContain(`任务 ${id}`);
    }
    expect(host.textContent).toContain("v1.2.3-task");
    expect(host.querySelector("[aria-current='true']")?.textContent).toContain("任务 level-4");
    expect(host.querySelectorAll("button[aria-expanded]")).toHaveLength(3);
  });

  it("lets parents collapse and opens a leaf in the shared inspector", () => {
    const onOpenTask = vi.fn();
    render(treeProject(), onOpenTask);
    const parentToggle = host.querySelector<HTMLButtonElement>(
      "button[aria-label='收起任务 任务 level-2']",
    );
    expect(parentToggle).not.toBeNull();

    act(() => parentToggle?.click());

    expect(host.textContent).not.toContain("任务 level-3");
    act(() => parentToggle?.click());
    const leafButton = [...host.querySelectorAll<HTMLButtonElement>(".tree-subtask-select")]
      .find(button => button.textContent?.includes("任务 level-4"));
    act(() => leafButton?.click());
    expect(onOpenTask).toHaveBeenCalledWith("level-4");
  });

  it("reopens a collapsed ancestor when a deep leaf becomes current", () => {
    render(treeProject());
    const rootToggle = host.querySelector<HTMLButtonElement>(
      "button[aria-label='收起任务 任务 level-1']",
    );
    act(() => rootToggle?.click());
    expect(host.textContent).not.toContain("任务 level-4");

    render(treeProject("level-4"));

    expect(host.textContent).toContain("任务 level-4");
    expect(host.querySelector("[aria-current='true']")?.textContent).toContain("任务 level-4");
  });
});
