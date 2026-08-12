import { describe, expect, it } from "vitest";
import type { ProjectStateChangedEvent, RuntimeSnapshot } from "./types";
import {
  advanceProjectSyncCursor,
  mergePendingProjectEvent,
  projectSyncFallbackIntervalMs,
  recoveryPresentationIsActive,
  shouldAcceptProjectStateEvent,
  shouldAcceptRuntimeSnapshot,
  shouldRequestRuntimeSnapshot,
  taskControlFallbackDecision,
  type ProjectSyncCursor,
} from "./projectSyncPolicy";

const cursor: ProjectSyncCursor = {
  projectName: "alpha",
  processStartId: "process-a",
  eventSequence: 8,
  dataRevision: 4,
  taskControlTreeRevision: 8,
  taskControlSnapshotVersion: "task-control-snapshot-v1",
  taskControlActionId: null,
  taskControlMode: "Shadow",
};

function event(sequence: number, overrides: Partial<ProjectStateChangedEvent> = {}): ProjectStateChangedEvent {
  return {
    project_name: "alpha",
    process_start_id: "process-a",
    event_sequence: sequence,
    data_revision: 4,
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
    occurred_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function snapshot(sequence: number, overrides: Partial<RuntimeSnapshot> = {}): RuntimeSnapshot {
  return {
    project: { name: "alpha" } as RuntimeSnapshot["project"],
    pipeline_state: null,
    process_start_id: "process-a",
    event_sequence: sequence,
    recovery_presentation: {
      kind: "None",
      title: "",
      reason: "",
      severity: "Info",
      primary_action: null,
      secondary_actions: [],
      preserve_current_code: true,
      requires_baseline_restore: false,
      supports_preview: false,
      automatic_retry: false,
      capabilities: [],
      decision_options: [],
      state_fingerprint: "none",
    },
    task_control_snapshot_version: "task-control-snapshot-v1",
    task_control_tree_revision: sequence,
    task_control_event_sequence: sequence,
    task_control_action_id: null,
    task_control_mode: "Shadow",
    ...overrides,
  };
}

describe("projectSyncPolicy", () => {
  it("rejects duplicate, out-of-order, and cross-project events", () => {
    expect(shouldAcceptProjectStateEvent(cursor, event(8))).toBe(false);
    expect(shouldAcceptProjectStateEvent(cursor, event(7))).toBe(false);
    expect(shouldAcceptProjectStateEvent(cursor, event(9, { project_name: "beta" }))).toBe(false);
    expect(shouldAcceptProjectStateEvent(cursor, event(9))).toBe(true);
  });

  it("resets ordering when the backend process changes", () => {
    const restarted = event(1, { process_start_id: "process-b" });
    expect(shouldAcceptProjectStateEvent(cursor, restarted)).toBe(true);
    expect(advanceProjectSyncCursor(cursor, restarted.process_start_id, restarted.event_sequence))
      .toEqual({
        ...cursor,
        processStartId: "process-b",
        eventSequence: 1,
        dataRevision: 0,
        taskControlTreeRevision: 0,
        taskControlSnapshotVersion: "",
        taskControlActionId: null,
        taskControlMode: null,
      });
  });

  it("does not request a full snapshot when event revisions do not advance", () => {
    expect(shouldRequestRuntimeSnapshot(cursor, event(9, {
      data_revision: 4,
      task_control_tree_revision: 8,
      task_control_dirty: false,
    }))).toBe(false);
    expect(shouldRequestRuntimeSnapshot(cursor, event(9, {
      data_revision: 5,
      task_control_tree_revision: 8,
      task_control_dirty: false,
    }))).toBe(true);
    expect(shouldRequestRuntimeSnapshot(cursor, event(9, {
      data_revision: 4,
      task_control_tree_revision: 8,
      task_control_dirty: false,
      runtime_dirty: true,
    }))).toBe(true);
  });

  it("uses a low-frequency fallback while the channel is healthy", () => {
    expect(projectSyncFallbackIntervalMs("connected", "synced")).toBe(60_000);
    expect(projectSyncFallbackIntervalMs("reconnecting", "delayed")).toBe(15_000);
    expect(projectSyncFallbackIntervalMs("connected", "disconnected")).toBe(15_000);
  });

  it("bounds active recovery sync without changing normal or disconnected fallback", () => {
    expect(projectSyncFallbackIntervalMs("connected", "synced", 60_000, true)).toBe(5_000);
    expect(projectSyncFallbackIntervalMs("connected", "synced", 60_000, false)).toBe(60_000);
    expect(projectSyncFallbackIntervalMs("reconnecting", "delayed", 60_000, true)).toBe(15_000);
    expect(recoveryPresentationIsActive({
      ...snapshot(1).recovery_presentation,
      progress_status: "stalled",
      background_retry_active: true,
    })).toBe(true);
    expect(recoveryPresentationIsActive({
      ...snapshot(1).recovery_presentation,
      progress_status: "waiting_human",
      background_retry_active: false,
    })).toBe(false);
  });

  it("rejects a snapshot older than the latest invalidation event", () => {
    expect(shouldAcceptRuntimeSnapshot(cursor, snapshot(7))).toBe(false);
    expect(shouldAcceptRuntimeSnapshot(cursor, snapshot(8))).toBe(true);
    expect(shouldAcceptRuntimeSnapshot(cursor, snapshot(9, {
      project: { name: "beta" } as RuntimeSnapshot["project"],
    }))).toBe(false);
  });

  it("coalesces same-process events without losing same-revision substate changes", () => {
    const first = event(9, { data_revision: 4 });
    const second = event(10, { data_revision: 4, execution_session_status: "session_lost" });
    expect(mergePendingProjectEvent(first, second)).toBe(second);
  });

  it("enables independent detail fallback only for abnormal sync facts", () => {
    const healthy = {
      enabled: true,
      subscriptionStatus: "connected" as const,
      runtimeSyncStatus: "synced" as const,
      detailStatus: "ready" as const,
      detailUpdatedAt: "2026-08-01T00:00:00Z",
      consecutiveFailures: 0,
      nowMs: Date.parse("2026-08-01T00:00:10Z"),
    };
    expect(taskControlFallbackDecision(healthy)).toEqual({ active: false, reason: null });
    expect(taskControlFallbackDecision({
      ...healthy,
      subscriptionStatus: "reconnecting",
    }).reason).toBe("channel_reconnecting");
    expect(taskControlFallbackDecision({
      ...healthy,
      detailStatus: "unavailable",
    }).reason).toBe("detail_unavailable");
    expect(taskControlFallbackDecision({
      ...healthy,
      nowMs: Date.parse("2026-08-01T00:01:00Z"),
    }).reason).toBe("detail_stale");
    expect(taskControlFallbackDecision({
      ...healthy,
      consecutiveFailures: 3,
    }).reason).toBe("sync_failures");
  });
});
