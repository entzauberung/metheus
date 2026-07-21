import { describe, expect, it } from "vitest";
import { executionPollingOwnsNextAdvance, getAutopilotErrorActions } from "./autopilotPolicy";
import type { PipelineState } from "./types";

function pipeline(status: PipelineState["status"]): PipelineState {
  return {
    execution_id: "execution-1",
    mid_stage_id: "mid-1",
    status,
    current_subtask_index: 0,
    total_subtasks: 1,
    subtask_statuses: [],
    current_log: "",
    child_pid: undefined,
    project_name: "project-1",
    milestone_id: "milestone-1",
    plan_revision: 1,
    current_subtask_id: "subtask-1",
    awaiting_confirmation: false,
    log_history: [],
  };
}

describe("autopilot scheduling policy", () => {
  it("delegates the next advance to polling while execution remains running", () => {
    expect(executionPollingOwnsNextAdvance(pipeline("Running"))).toBe(true);
    expect(executionPollingOwnsNextAdvance(pipeline("Completed"))).toBe(false);
  });

  it("keeps a close action for every stopped recovery category", () => {
    for (const recovery of [
      "None",
      "RestoreExecutionBaseline",
      "RetryAutopilotAdvance",
      "SyncAndClose",
      "WaitHumanDecision",
      "RegenerateExecutionPlan",
      "PrepareExecutionWorkspace",
      "ResolveWorkspaceChanges",
      "RunAutomaticRecovery",
    ] as const) {
      expect(getAutopilotErrorActions("ErrorStopped", recovery).canClose).toBe(true);
    }
  });

  it("does not expose a manual resume while automatic recovery owns the next action", () => {
    expect(getAutopilotErrorActions("ErrorStopped", "RunAutomaticRecovery")).toMatchObject({
      canResume: false,
      canRetryAdvance: false,
      canClose: true,
    });
  });

  it("only exposes retry advance for retryable infrastructure errors", () => {
    expect(getAutopilotErrorActions("ErrorStopped", "RetryAutopilotAdvance")).toMatchObject({
      canResume: false,
      canRetryAdvance: true,
    });
    expect(getAutopilotErrorActions("ErrorStopped", "WaitHumanDecision")).toMatchObject({
      canResume: false,
      canRetryAdvance: false,
      canClose: true,
    });
  });

  it("maps deterministic preconditions to one explicit recovery action", () => {
    expect(getAutopilotErrorActions("ErrorStopped", "RegenerateExecutionPlan")).toMatchObject({
      canRegeneratePlan: true,
      canPrepareWorkspace: false,
      canRefreshWorkspace: false,
    });
    expect(getAutopilotErrorActions("ErrorStopped", "PrepareExecutionWorkspace")).toMatchObject({
      canRegeneratePlan: false,
      canPrepareWorkspace: true,
      canRefreshWorkspace: false,
    });
    expect(getAutopilotErrorActions("ErrorStopped", "ResolveWorkspaceChanges")).toMatchObject({
      canRegeneratePlan: false,
      canPrepareWorkspace: false,
      canRefreshWorkspace: true,
    });
  });
});
