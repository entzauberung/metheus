import { describe, expect, it } from "vitest";
import type { PipelineState } from "./types";
import {
  executionPollDecision,
  isTerminalRuntimeSnapshot,
  shouldReconcileAfterPollFailure,
  terminalDelayedSyncDelay,
  terminalSyncDelay,
  TERMINAL_SYNC_MAX_WAIT_MS,
} from "./executionSyncPolicy";
import type { RuntimeSnapshot } from "./types";

function pipeline(overrides: Partial<PipelineState> = {}): PipelineState {
  return {
    execution_id: "execution-1",
    mid_stage_id: "mid-1",
    status: "Running",
    current_subtask_index: 0,
    total_subtasks: 1,
    subtask_statuses: [],
    current_log: "running",
    project_name: "alpha",
    milestone_id: "milestone-1",
    plan_revision: 1,
    current_subtask_id: "task-1",
    awaiting_confirmation: false,
    log_history: [],
    ...overrides,
  };
}

describe("executionSyncPolicy", () => {
  it("continues polling only for the current running execution", () => {
    expect(executionPollDecision(pipeline(), "alpha")).toBe("continue");
    expect(executionPollDecision(pipeline({ project_name: "beta" }), "alpha")).toBe("reconcile");
  });

  it("reconciles missing, failed, completed, and awaiting-confirmation states", () => {
    expect(executionPollDecision(null, "alpha")).toBe("reconcile");
    expect(executionPollDecision(pipeline({ status: "Failed" }), "alpha")).toBe("reconcile");
    expect(executionPollDecision(pipeline({ status: "Completed" }), "alpha")).toBe("reconcile");
    expect(executionPollDecision(pipeline({ awaiting_confirmation: true }), "alpha")).toBe("reconcile");
  });

  it("uses finite terminal reconciliation backoff", () => {
    expect(terminalSyncDelay(0)).toBe(0);
    expect(terminalSyncDelay(1)).toBe(250);
    expect(terminalSyncDelay(2)).toBe(750);
    expect(terminalSyncDelay(3)).toBeNull();
    expect(terminalDelayedSyncDelay(0)).toBe(1_500);
    expect(terminalDelayedSyncDelay(2)).toBe(5_000);
    expect(terminalDelayedSyncDelay(3)).toBeNull();
    expect(TERMINAL_SYNC_MAX_WAIT_MS).toBe(10_500);
  });

  it("reconciles after consecutive poll transport failures", () => {
    expect(shouldReconcileAfterPollFailure(1)).toBe(false);
    expect(shouldReconcileAfterPollFailure(2)).toBe(true);
  });

  it("accepts a terminal snapshot only after durable and runtime execution facts agree", () => {
    const runtime = (status: PipelineState | null, session?: { active: boolean; status: string }) => ({
      project: {
        name: "alpha",
        execution_session: session,
      },
      pipeline_state: status,
      process_start_id: "process-1",
      event_sequence: 3,
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
    } as unknown as RuntimeSnapshot);

    expect(isTerminalRuntimeSnapshot(runtime(pipeline()), "alpha")).toBe(false);
    expect(isTerminalRuntimeSnapshot(runtime(null, { active: true, status: "executing" }), "alpha"))
      .toBe(false);
    expect(isTerminalRuntimeSnapshot(runtime(pipeline({ status: "Failed" }), {
      active: false,
      status: "execution_failed",
    }), "alpha")).toBe(true);
    expect(isTerminalRuntimeSnapshot(runtime(null), "alpha")).toBe(true);
  });
});
