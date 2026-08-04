/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ProjectStateChangedEvent,
  ProjectStateSubscription,
  RecoveryPresentation,
  RuntimeSnapshot,
  TaskControlSnapshot,
} from "../types";
import {
  useProjectStateSync,
  type ProjectStateSyncController,
  type ProjectStateSyncTransport,
} from "./useProjectStateSync";

function recovery(fingerprint = "none"): RecoveryPresentation {
  return {
    kind: fingerprint === "none" ? "None" : "BaselineRecovery",
    title: fingerprint === "none" ? "" : "执行已阻断",
    reason: fingerprint === "none" ? "" : "执行会话已经丢失",
    severity: fingerprint === "none" ? "Info" : "Error",
    primary_action: null,
    secondary_actions: [],
    preserve_current_code: fingerprint === "none",
    requires_baseline_restore: fingerprint !== "none",
    supports_preview: fingerprint !== "none",
    automatic_retry: false,
    capabilities: [],
    decision_options: [],
    state_fingerprint: fingerprint,
  };
}

function snapshot(
  projectName: string,
  sequence: number,
  fingerprint = "none",
  taskTreeRevision = sequence,
): RuntimeSnapshot {
  return {
    project: {
      name: projectName,
      workflow_state: { data_revision: sequence },
    } as RuntimeSnapshot["project"],
    pipeline_state: null,
    process_start_id: "process-1",
    event_sequence: sequence,
    recovery_presentation: recovery(fingerprint),
    task_control_snapshot_version: "task-control-snapshot-v1",
    task_control_tree_revision: taskTreeRevision,
    task_control_event_sequence: sequence,
    task_control_action_id: null,
    task_control_mode: "Shadow",
  };
}

function event(sequence: number, overrides: Partial<ProjectStateChangedEvent> = {}): ProjectStateChangedEvent {
  return {
    project_name: "alpha",
    process_start_id: "process-1",
    event_sequence: sequence,
    data_revision: 1,
    current_step: "Execution",
    execution_session_status: "execution_failed",
    autopilot_status: null,
    recovery_action: "RestoreExecutionBaseline",
    task_control_tree_revision: sequence,
    task_control_snapshot_version: "task-control-snapshot-v1",
    control_action_id: null,
    control_mode: "Shadow",
    task_control_dirty: true,
    runtime_dirty: false,
    occurred_at: "2026-07-31T00:00:00Z",
    ...overrides,
  };
}

describe("useProjectStateSync", () => {
  let host: HTMLDivElement;
  let root: Root;
  let controller: ProjectStateSyncController | null;
  let channels: Array<{ onmessage: (value: ProjectStateChangedEvent) => void }>;
  let subscribe: ReturnType<typeof vi.fn>;
  let unsubscribe: ReturnType<typeof vi.fn>;
  let getSnapshot: ReturnType<typeof vi.fn>;
  let transport: ProjectStateSyncTransport;
  let onSnapshot: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    controller = null;
    channels = [];
    subscribe = vi.fn(async (_projectName: string): Promise<ProjectStateSubscription> => ({
      subscription_id: `subscription-${_projectName}`,
      process_start_id: "process-1",
      event_sequence: 0,
    }));
    unsubscribe = vi.fn(async () => undefined);
    getSnapshot = vi.fn();
    transport = {
      createChannel(onmessage) {
        const channel = { onmessage };
        channels.push(channel);
        return channel;
      },
      subscribe,
      unsubscribe,
      getSnapshot,
    };
    onSnapshot = vi.fn();
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  function render(projectName: string, includeTaskControlSnapshot = false) {
    function Harness() {
      controller = useProjectStateSync({
        projectName,
        includeTaskControlSnapshot,
        onSnapshot,
        coalesceMs: 10,
        fallbackIntervalMs: 60_000,
        transport,
      });
      return null;
    }
    act(() => root.render(<Harness />));
  }

  async function flush() {
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  function lastAppliedSnapshot(): RuntimeSnapshot | undefined {
    return onSnapshot.mock.calls[onSnapshot.mock.calls.length - 1]?.[0];
  }

  it("coalesces notifications and applies the newest runtime snapshot", async () => {
    getSnapshot
      .mockResolvedValueOnce(snapshot("alpha", 0))
      .mockResolvedValueOnce(snapshot("alpha", 2, "blocked-2"));
    render("alpha");
    await flush();

    act(() => {
      channels[0].onmessage(event(1));
      channels[0].onmessage(event(2));
    });
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(getSnapshot).toHaveBeenCalledTimes(2);
    expect(onSnapshot).toHaveBeenCalledTimes(2);
    expect(lastAppliedSnapshot()?.recovery_presentation.state_fingerprint)
      .toBe("blocked-2");
    expect(controller?.state.lastEventSequence).toBe(2);

    act(() => channels[0].onmessage(event(1)));
    act(() => vi.advanceTimersByTime(10));
    expect(getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("does not fetch a full snapshot when an event carries no newer revision", async () => {
    getSnapshot.mockResolvedValue(snapshot("alpha", 1, "none", 1));
    render("alpha");
    await flush();

    act(() => channels[0].onmessage(event(2, {
      data_revision: 1,
      task_control_tree_revision: 1,
      task_control_dirty: false,
    })));
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
    });

    expect(getSnapshot).toHaveBeenCalledTimes(1);
    expect(controller?.state.lastEventSequence).toBe(2);
    expect(controller?.state.pendingRevision).toBeNull();
  });

  it("keeps the connected fallback at sixty seconds", async () => {
    getSnapshot.mockResolvedValue(snapshot("alpha", 0));
    render("alpha");
    await flush();

    await act(async () => {
      vi.advanceTimersByTime(15_000);
      await Promise.resolve();
    });
    expect(getSnapshot).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(45_000);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(getSnapshot).toHaveBeenCalledTimes(2);
  });

  it("requests a same-cursor detailed snapshot only while the inspector asks for it", async () => {
    getSnapshot.mockImplementation(async (_projectName: string, includeDetail: boolean) => {
      const next = snapshot("alpha", 0);
      if (includeDetail) {
        next.task_control_snapshot = {
          project_name: "alpha",
          project_revision: 0,
          task_tree_revision: 0,
          source_process_start_id: "process-1",
          source_event_sequence: 0,
          source_control_action_id: null,
        } as TaskControlSnapshot;
      }
      return next;
    });

    function Harness({ includeDetail }: { includeDetail: boolean }) {
      controller = useProjectStateSync({
        projectName: "alpha",
        includeTaskControlSnapshot: includeDetail,
        onSnapshot,
        coalesceMs: 10,
        fallbackIntervalMs: 60_000,
        transport,
      });
      return null;
    }

    act(() => root.render(<Harness includeDetail={false} />));
    await flush();
    expect(getSnapshot).toHaveBeenLastCalledWith("alpha", false);

    act(() => root.render(<Harness includeDetail />));
    await flush();
    expect(getSnapshot).toHaveBeenLastCalledWith("alpha", true);
    expect(getSnapshot).toHaveBeenCalledTimes(2);
    expect(controller?.state.taskControlDetailStatus).toBe("ready");
  });

  it("invalidates task control only for task-control events", async () => {
    getSnapshot
      .mockResolvedValueOnce(snapshot("alpha", 0))
      .mockResolvedValueOnce(snapshot("alpha", 2));
    render("alpha");
    await flush();

    act(() => channels[0].onmessage(event(1, { task_control_dirty: false })));
    expect(controller?.state.taskControlEventSequence).toBe(0);
    act(() => channels[0].onmessage(event(2, {
      task_control_dirty: true,
      task_control_tree_revision: 8,
    })));
    expect(controller?.state.taskControlEventSequence).toBe(2);
    expect(controller?.state.taskControlTreeRevision).toBe(8);
  });

  it("does not advance task-control invalidation after a project-only snapshot", async () => {
    getSnapshot
      .mockResolvedValueOnce(snapshot("alpha", 0, "none", 4))
      .mockResolvedValueOnce(snapshot("alpha", 1, "none", 4));
    render("alpha");
    await flush();

    act(() => channels[0].onmessage(event(1, {
      task_control_dirty: false,
      task_control_tree_revision: 4,
    })));
    await act(async () => {
      vi.advanceTimersByTime(10);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(controller?.state.taskControlEventSequence).toBe(0);
    expect(controller?.state.taskControlTreeRevision).toBe(4);
  });

  it("isolates project switches and rejects the old asynchronous response", async () => {
    let resolveAlpha: ((value: RuntimeSnapshot) => void) | undefined;
    getSnapshot.mockImplementation((projectName: string) => {
      if (projectName === "alpha") {
        return new Promise<RuntimeSnapshot>(resolve => { resolveAlpha = resolve; });
      }
      return Promise.resolve(snapshot("beta", 1));
    });

    render("alpha");
    await flush();
    render("beta");
    await flush();
    expect(lastAppliedSnapshot()?.project.name).toBe("beta");

    await act(async () => {
      resolveAlpha?.(snapshot("alpha", 9, "stale-alpha"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(lastAppliedSnapshot()?.project.name).toBe("beta");
    expect(unsubscribe).toHaveBeenCalledWith("subscription-alpha");
  });

  it("preserves the last snapshot and reports a disconnected backend", async () => {
    getSnapshot
      .mockResolvedValueOnce(snapshot("alpha", 1))
      .mockRejectedValueOnce(new Error("offline-1"))
      .mockRejectedValueOnce(new Error("offline-2"))
      .mockRejectedValueOnce(new Error("offline-3"));
    render("alpha");
    await flush();

    await act(async () => { await controller?.forceSync(); });
    await act(async () => { await controller?.forceSync(); });
    await act(async () => { await controller?.forceSync(); });

    expect(onSnapshot).toHaveBeenCalledTimes(1);
    expect(controller?.state.status).toBe("disconnected");
    expect(controller?.state.lastSuccessfulSyncAt).not.toBeNull();
    expect(controller?.state.consecutiveFailures).toBe(3);
  });

  it("waits for the newest queued snapshot when force sync overlaps an in-flight request", async () => {
    getSnapshot.mockResolvedValueOnce(snapshot("alpha", 0));
    render("alpha");
    await flush();

    let resolveFirst: ((value: RuntimeSnapshot) => void) | undefined;
    getSnapshot
      .mockImplementationOnce(() => new Promise<RuntimeSnapshot>(resolve => { resolveFirst = resolve; }))
      .mockResolvedValueOnce(snapshot("alpha", 2, "latest"));

    let first!: Promise<RuntimeSnapshot | null>;
    let second!: Promise<RuntimeSnapshot | null>;
    act(() => {
      first = controller!.forceSync();
      second = controller!.forceSync();
    });
    await act(async () => {
      resolveFirst?.(snapshot("alpha", 1, "intermediate"));
      await first;
      await second;
    });

    expect(getSnapshot).toHaveBeenCalledTimes(3);
    expect((await second)?.event_sequence).toBe(2);
    expect(lastAppliedSnapshot()?.recovery_presentation.state_fingerprint)
      .toBe("latest");
  });

  it("keeps snapshot fallback delayed until a failed channel subscription reconnects", async () => {
    subscribe
      .mockRejectedValueOnce(new Error("channel offline"))
      .mockResolvedValueOnce({
        subscription_id: "subscription-alpha-2",
        process_start_id: "process-1",
        event_sequence: 0,
      });
    getSnapshot.mockResolvedValue(snapshot("alpha", 0));

    render("alpha");
    await flush();
    expect(controller?.state.status).toBe("delayed");
    expect(controller?.state.subscriptionStatus).toBe("reconnecting");

    await act(async () => {
      vi.advanceTimersByTime(1_000);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(subscribe).toHaveBeenCalledTimes(2);
    expect(controller?.state.subscriptionStatus).toBe("connected");
    expect(controller?.state.status).toBe("synced");
  });
});
