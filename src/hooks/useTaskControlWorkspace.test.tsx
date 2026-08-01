/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, RuntimeMutationResult, TaskControlSnapshot } from "../types";
import {
  isTaskControlSnapshotCurrent,
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
    snapshot_version: "task-control-snapshot-v1",
    project_name: name,
    project_revision: revision,
    current_task_id: currentTaskId,
    task_tree_revision: revision,
    source_process_start_id: "process-1",
    source_event_sequence: revision,
    source_control_action_id: null,
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

function mutation(name: string, revision: number, control: TaskControlSnapshot): RuntimeMutationResult {
  return {
    result_version: "runtime-mutation-v1",
    runtime_snapshot: {
      project: project(name, revision),
      pipeline_state: null,
      process_start_id: "process-1",
      event_sequence: revision,
      recovery_presentation: { kind: "None" },
      task_control_snapshot_version: "task-control-snapshot-v1",
      task_control_tree_revision: revision,
      task_control_event_sequence: revision,
      task_control_action_id: null,
      task_control_mode: "Shadow",
    },
    task_control: {
      available: true,
      snapshot_version: "task-control-snapshot-v1",
      tree_revision: revision,
      event_sequence: revision,
      control_action_id: null,
      control_mode: "Shadow",
    },
    action: { action: "test", message: "", notify_user: false, recovery_result: null },
    task_control_snapshot: control,
  } as unknown as RuntimeMutationResult;
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

  function render(source: Project, onRuntimeMutation = vi.fn()) {
    function Harness() {
      workspace = useTaskControlWorkspace({
        project: source,
        pollIntervalMs: 1_000,
        onRuntimeMutation,
      });
      return null;
    }
    act(() => root.render(<Harness />));
  }

  it("uses one fallback poller and keeps a valid selection across refreshes", async () => {
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

  it("applies the unified action result without a compensating project fetch", async () => {
    const onRuntimeMutation = vi.fn();
    const updatedSnapshot = snapshot("alpha", 2, "task-a");
    invokeHarness.invoke
      .mockResolvedValueOnce(snapshot("alpha", 1, "task-a"))
      .mockResolvedValueOnce(mutation("alpha", 2, updatedSnapshot));
    render(project("alpha", 1), onRuntimeMutation);
    await act(async () => { await Promise.resolve(); });

    await act(async () => { await workspace?.executeAction("revalidate"); });
    expect(onRuntimeMutation).toHaveBeenCalledTimes(1);
    expect(workspace?.selectedTaskId).toBe("task-a");
    expect(workspace?.snapshot?.project_revision).toBe(2);
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(2);
  });

  it("applies a detailed snapshot carried by the same runtime cursor without a second request", async () => {
    const atomic = snapshot("alpha", 4, "task-atomic");
    const cursor = {
      processStartId: "process-1",
      eventSequence: 4,
      projectRevision: 4,
      treeRevision: 4,
      controlActionId: null,
      controlActionKnown: true,
      snapshotVersion: "task-control-snapshot-v1",
    };

    function Harness() {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 4),
        runtimeCursor: cursor,
        atomicSnapshot: atomic,
        atomicSnapshotStatus: "ready",
        atomicSnapshotUpdatedAt: new Date().toISOString(),
      });
      return null;
    }

    act(() => root.render(<Harness />));
    await act(async () => { await Promise.resolve(); });
    expect(workspace?.snapshot?.current_task_id).toBe("task-atomic");
    expect(workspace?.detailsSyncing).toBe(false);
    expect(invokeHarness.invoke).not.toHaveBeenCalled();
  });

  it("waits for the atomic runtime detail before using the independent fallback", async () => {
    invokeHarness.invoke.mockResolvedValue(snapshot("alpha", 4, "fallback-task"));
    const cursor = {
      processStartId: "process-1",
      eventSequence: 4,
      projectRevision: 4,
      treeRevision: 4,
      controlActionId: null,
      controlActionKnown: true,
      snapshotVersion: "task-control-snapshot-v1",
    };

    function Harness({ status }: { status: "syncing" | "unavailable" }) {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 4),
        runtimeCursor: cursor,
        atomicSnapshotStatus: status,
      });
      return null;
    }

    act(() => root.render(<Harness status="syncing" />));
    await act(async () => { await Promise.resolve(); });
    expect(workspace?.detailsSyncing).toBe(true);
    expect(invokeHarness.invoke).not.toHaveBeenCalled();

    act(() => root.render(<Harness status="unavailable" />));
    await act(async () => { await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(1);
    expect(workspace?.snapshot?.current_task_id).toBe("fallback-task");
  });

  it("sends task actions to the user-selected node", async () => {
    const initial = snapshot("alpha", 1, "task-a");
    initial.nodes.push(snapshot("alpha", 1, "task-b").nodes[0]);
    invokeHarness.invoke
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce(mutation("alpha", 2, snapshot("alpha", 2, "task-a")));
    render(project("alpha", 1));
    await act(async () => { await Promise.resolve(); });
    act(() => workspace?.selectTask("task-b"));

    await act(async () => { await workspace?.executeAction("revalidate"); });

    expect(invokeHarness.invoke.mock.calls[1][1].request.task_id).toBe("task-b");
  });

  it("refreshes immediately for an invalidation sequence and preserves selection", async () => {
    const initial = snapshot("alpha", 1, "task-a");
    const invalidated = snapshot("alpha", 2, "task-b");
    invalidated.source_event_sequence = 12;
    invalidated.nodes.push(snapshot("alpha", 2, "task-a").nodes[0]);
    invokeHarness.invoke
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce(invalidated);
    const source = project("alpha", 1);

    function Harness({ sequence }: { sequence: number }) {
      workspace = useTaskControlWorkspace({
        project: source,
        invalidationSequence: sequence,
      });
      return null;
    }

    act(() => root.render(<Harness sequence={0} />));
    await act(async () => { await Promise.resolve(); });
    act(() => workspace?.selectTask("task-a"));
    act(() => root.render(<Harness sequence={12} />));
    await act(async () => { await Promise.resolve(); });

    expect(invokeHarness.invoke).toHaveBeenCalledTimes(2);
    expect(workspace?.sourceEventSequence).toBe(12);
    expect(workspace?.selectedTaskId).toBe("task-a");
  });

  it("uses bounded short retries when detailed snapshot generation fails", async () => {
    invokeHarness.invoke
      .mockRejectedValueOnce(new Error("detail generation failed"))
      .mockRejectedValueOnce(new Error("detail generation failed"))
      .mockResolvedValueOnce(snapshot("alpha", 1));

    function Harness() {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 1),
        pollIntervalMs: 30_000,
      });
      return null;
    }

    act(() => root.render(<Harness />));
    await act(async () => { await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(1);
    expect(workspace?.detailsSyncing).toBe(true);
    expect(workspace?.error).toContain("detail generation failed");

    await act(async () => { vi.advanceTimersByTime(1_000); await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(2);
    await act(async () => { vi.advanceTimersByTime(3_000); await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(3);
    expect(workspace?.snapshot?.project_revision).toBe(1);
    expect(workspace?.detailsSyncing).toBe(false);
    expect(workspace?.error).toBe("");

    await act(async () => { vi.advanceTimersByTime(10_000); await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(3);
  });

  it("uses a low-frequency fallback interval", async () => {
    const interval = vi.spyOn(window, "setInterval");
    invokeHarness.invoke.mockResolvedValueOnce(snapshot("alpha", 1));

    function Harness() {
      workspace = useTaskControlWorkspace({ project: project("alpha", 1) });
      return null;
    }

    act(() => root.render(<Harness />));
    await act(async () => { await Promise.resolve(); });
    expect(interval).toHaveBeenCalledWith(expect.any(Function), 30_000);
  });

  it("does not create an independent poller while atomic detail and channel are healthy", async () => {
    const interval = vi.spyOn(window, "setInterval");
    const atomic = snapshot("alpha", 4, "task-atomic");
    function Harness() {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 4),
        atomicSnapshot: atomic,
        atomicSnapshotStatus: "ready",
        atomicSnapshotUpdatedAt: new Date().toISOString(),
        subscriptionStatus: "connected",
        runtimeSyncStatus: "synced",
      });
      return null;
    }
    act(() => root.render(<Harness />));
    await act(async () => { await Promise.resolve(); });
    expect(interval).not.toHaveBeenCalled();
    expect(invokeHarness.invoke).not.toHaveBeenCalled();
    expect(workspace?.detailFallbackActive).toBe(false);
  });

  it("starts fallback for a channel fault and stops it after recovery", async () => {
    const atomic = snapshot("alpha", 4, "task-atomic");
    invokeHarness.invoke.mockResolvedValue(snapshot("alpha", 4, "task-atomic"));
    function Harness({ channel }: { channel: "connected" | "reconnecting" }) {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 4),
        pollIntervalMs: 1_000,
        atomicSnapshot: atomic,
        atomicSnapshotStatus: "ready",
        atomicSnapshotUpdatedAt: new Date().toISOString(),
        subscriptionStatus: channel,
        runtimeSyncStatus: "synced",
      });
      return null;
    }
    act(() => root.render(<Harness channel="connected" />));
    await act(async () => { await Promise.resolve(); });
    expect(workspace?.detailFallbackActive).toBe(false);

    act(() => root.render(<Harness channel="reconnecting" />));
    expect(workspace?.detailFallbackReason).toBe("channel_reconnecting");
    await act(async () => { vi.advanceTimersByTime(1_000); await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(1);

    act(() => root.render(<Harness channel="connected" />));
    expect(workspace?.detailFallbackActive).toBe(false);
    await act(async () => { vi.advanceTimersByTime(5_000); await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(1);
  });

  it("stops all detailed snapshot requests while the inspector is closed", async () => {
    const interval = vi.spyOn(window, "setInterval");

    function Harness() {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 1),
        enabled: false,
      });
      return null;
    }

    act(() => root.render(<Harness />));
    await act(async () => { await Promise.resolve(); });
    expect(invokeHarness.invoke).not.toHaveBeenCalled();
    expect(interval).not.toHaveBeenCalled();
  });

  it("cancels a pending short retry when the inspector closes", async () => {
    invokeHarness.invoke.mockRejectedValueOnce(new Error("detail generation failed"));

    function Harness({ enabled }: { enabled: boolean }) {
      workspace = useTaskControlWorkspace({
        project: project("alpha", 1),
        enabled,
      });
      return null;
    }

    act(() => root.render(<Harness enabled />));
    await act(async () => { await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(1);

    act(() => root.render(<Harness enabled={false} />));
    await act(async () => { vi.advanceTimersByTime(5_000); await Promise.resolve(); });
    expect(invokeHarness.invoke).toHaveBeenCalledTimes(1);
  });

  it("rejects detailed snapshots behind any runtime control cursor", () => {
    const current = snapshot("alpha", 8);
    current.task_tree_revision = 5;
    current.source_event_sequence = 8;
    current.source_control_action_id = "action-8";
    const cursor = {
      processStartId: "process-1",
      eventSequence: 8,
      projectRevision: 8,
      treeRevision: 5,
      controlActionId: "action-8",
      controlActionKnown: true,
      snapshotVersion: "task-control-snapshot-v1",
    };
    expect(isTaskControlSnapshotCurrent(current, cursor)).toBe(true);
    expect(isTaskControlSnapshotCurrent({ ...current, source_process_start_id: "old" }, cursor)).toBe(false);
    expect(isTaskControlSnapshotCurrent({ ...current, source_event_sequence: 7 }, cursor)).toBe(false);
    expect(isTaskControlSnapshotCurrent({ ...current, project_revision: 7 }, cursor)).toBe(false);
    expect(isTaskControlSnapshotCurrent({ ...current, task_tree_revision: 4 }, cursor)).toBe(false);
    expect(isTaskControlSnapshotCurrent({ ...current, source_control_action_id: "old" }, cursor)).toBe(false);
  });
});
