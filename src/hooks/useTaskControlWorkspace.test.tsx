/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, TaskControlSnapshot } from "../types";
import {
  useTaskControlWorkspace,
  type TaskControlWorkspace,
} from "./useTaskControlWorkspace";

const invokeHarness = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("../utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeHarness.invoke,
}));

function project(name: string, revision: number): Project {
  return {
    name,
    milestones: [],
    workflow_state: { data_revision: revision },
  } as unknown as Project;
}

function snapshot(name: string, revision: number, currentTaskId = "task-a"): TaskControlSnapshot {
  return {
    project_name: name,
    project_revision: revision,
    current_task_id: currentTaskId,
    task_tree_revision: revision,
    control_mode: "Shadow",
    nodes: [{
      id: currentTaskId,
      title: currentTaskId,
      node_type: "Subtask",
      status: "Pending",
      depth: 1,
      complexity: "Small",
      risk: "Low",
      contract_fingerprint: "",
      dependencies: [],
      acceptance: [],
      children: [],
    }],
    control_capabilities: [],
    events: [],
  } as unknown as TaskControlSnapshot;
}

describe("useTaskControlWorkspace", () => {
  let host: HTMLDivElement;
  let root: Root;
  let workspace: TaskControlWorkspace | null;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    invokeHarness.invoke.mockReset();
    workspace = null;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function render(source: Project, onProjectUpdated = vi.fn()) {
    function Harness() {
      workspace = useTaskControlWorkspace({
        project: source,
        pollIntervalMs: 1_000,
        onProjectUpdated,
      });
      return null;
    }
    act(() => root.render(<Harness />));
  }

  it("uses one poller and keeps a valid selection across refreshes", async () => {
    const interval = vi.spyOn(window, "setInterval");
    const refreshed = snapshot("alpha", 2, "task-b");
    refreshed.nodes.push(snapshot("alpha", 2, "task-a").nodes[0]);
    invokeHarness.invoke
      .mockResolvedValueOnce(snapshot("alpha", 1, "task-a"))
      .mockResolvedValueOnce(refreshed);

    render(project("alpha", 1));
    await act(async () => { await Promise.resolve(); });
    expect(interval).toHaveBeenCalledTimes(1);
    expect(workspace?.selectedTaskId).toBe("task-a");

    act(() => workspace?.selectTask("task-a"));
    await act(async () => { vi.advanceTimersByTime(1_000); await Promise.resolve(); });
    expect(workspace?.selectedTaskId).toBe("task-a");
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(2);
  });

  it("falls back to the backend current task when the selection disappears", async () => {
    invokeHarness.invoke
      .mockResolvedValueOnce(snapshot("alpha", 1, "task-a"))
      .mockResolvedValueOnce(snapshot("alpha", 2, "task-b"));
    render(project("alpha", 1));
    await act(async () => { await Promise.resolve(); });
    act(() => workspace?.selectTask("missing"));
    await act(async () => { await workspace?.refresh(); });
    expect(workspace?.selectedTaskId).toBe("task-b");
  });

  it("does not let an old project response replace a newer project", async () => {
    let resolveOld: ((value: TaskControlSnapshot) => void) | undefined;
    invokeHarness.invoke.mockImplementationOnce(() => new Promise(resolve => {
      resolveOld = resolve;
    }));
    render(project("alpha", 1));

    invokeHarness.invoke.mockResolvedValueOnce(snapshot("beta", 5, "beta-task"));
    render(project("beta", 5));
    await act(async () => { await Promise.resolve(); });
    expect(workspace?.snapshot?.project_name).toBe("beta");

    await act(async () => { resolveOld?.(snapshot("alpha", 1)); await Promise.resolve(); });
    expect(workspace?.snapshot?.project_name).toBe("beta");
    expect(workspace?.selectedTaskId).toBe("beta-task");
  });

  it("does not let an old revision replace a newer revision of the same project", async () => {
    let resolveOld: ((value: TaskControlSnapshot) => void) | undefined;
    invokeHarness.invoke.mockImplementationOnce(() => new Promise(resolve => {
      resolveOld = resolve;
    }));
    render(project("alpha", 1));

    invokeHarness.invoke.mockResolvedValueOnce(snapshot("alpha", 5, "new-task"));
    render(project("alpha", 5));
    await act(async () => { await Promise.resolve(); });
    expect(workspace?.snapshot?.project_revision).toBe(5);

    await act(async () => { resolveOld?.(snapshot("alpha", 1, "old-task")); await Promise.resolve(); });
    expect(workspace?.snapshot?.project_revision).toBe(5);
    expect(workspace?.selectedTaskId).toBe("new-task");
  });

  it("refreshes project and snapshot after an action without losing selection on failure", async () => {
    const updated = project("alpha", 2);
    const onProjectUpdated = vi.fn();
    invokeHarness.invoke
      .mockResolvedValueOnce(snapshot("alpha", 1, "task-a"))
      .mockResolvedValueOnce({ snapshot: snapshot("alpha", 2, "task-a") })
      .mockResolvedValueOnce(updated)
      .mockRejectedValueOnce(new Error("snapshot unavailable"));
    render(project("alpha", 1), onProjectUpdated);
    await act(async () => { await Promise.resolve(); });

    await act(async () => { await workspace?.executeAction("revalidate"); });
    expect(onProjectUpdated).toHaveBeenCalledWith(updated);
    expect(workspace?.selectedTaskId).toBe("task-a");
    expect(workspace?.error).toContain("snapshot unavailable");
  });
});
