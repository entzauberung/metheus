import { describe, expect, it } from "vitest";
import {
  executionPollingOwnsNextAdvance,
  getAutopilotErrorActions,
  getGitConfirmationBlockPresentation,
  getQualityStatusPresentation,
  getRecoveryStatusLabel,
} from "./autopilotPolicy";
import type { PipelineState, RecoveryState } from "./types";

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

function recovery(overrides: Partial<RecoveryState>): RecoveryState {
  return {
    error_kind: "EvidenceInsufficient",
    phase: "Retesting",
    attempt: 0,
    max_attempts: 2,
    error_signature: "",
    repeated_signature_count: 1,
    subtask_id: "subtask-1",
    execution_id: "execution-1",
    baseline_commit: "abc123",
    last_diagnosis: "",
    last_repair_summary: "",
    original_test_failure: "",
    replan_attempted: false,
    failure_history: [],
    active_issues: [],
    attempt_history: [],
    replan_execution_attempted: false,
    started_at: "",
    updated_at: "",
    checkpoint_id: "",
    rollback_retest_pending: false,
    evidence_rebuild_attempted: false,
    evidence_rebuild_attempts: 0,
    pending_evidence_criteria: [1],
    evidence_strategies: [],
    ...overrides,
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
      "RetryGitConfirmation",
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

  it("routes Git confirmation blocks only to confirmation retry", () => {
    expect(getAutopilotErrorActions("ErrorStopped", "RetryGitConfirmation")).toMatchObject({
      canResume: false,
      canRetryAdvance: false,
      canRetryGitConfirmation: true,
    });
  });

  it("only allows retry for recoverable Git confirmation failures", () => {
    for (const failureKind of [
      "LegacyV1TagConflict",
      "CommitFailed",
      "TagFailed",
      "ProjectFinalizationFailed",
      "GitMetadataUnavailable",
    ] as const) {
      expect(getGitConfirmationBlockPresentation(failureKind).canRetry).toBe(true);
    }

    for (const failureKind of [
      "V2TagIntegrityConflict",
      "TagIdentityConflict",
      "ScopeViolation",
      undefined,
    ] as const) {
      expect(getGitConfirmationBlockPresentation(failureKind).canRetry).toBe(false);
    }
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

  it("distinguishes evidence rebuilding from exhausted code recovery", () => {
    expect(getRecoveryStatusLabel(recovery({ evidence_rebuild_attempts: 1 })))
      .toContain("正在补充验收证据");
    expect(getRecoveryStatusLabel(recovery({ phase: "WaitingHuman" })))
      .toBe("验收证据仍不足，等待人工处理");
    expect(getRecoveryStatusLabel(recovery({
      phase: "WaitingHuman",
      error_kind: "ReviewFailure",
      attempt: 2,
    }))).toBe("自动恢复已耗尽，等待人工处理");
  });

  it("presents automated tests, code review, and acceptance evidence separately", () => {
    const presentation = getQualityStatusPresentation({
      passed: true,
      issues: [],
      suggestion: "",
      automated_test_status: "NotConfigured",
      review_passed: true,
      review_evidence_status: "Partial",
      review_issues: [{
        criterion: "criterion",
        file: "index.html",
        expected: "",
        actual: "style suggestion",
        suggested_change: "",
        confidence: 0.9,
        severity: "Suggestion",
      }],
    }, [{
      criterion_index: 1,
      criterion: "criterion",
      status: "Satisfied",
      evidence: "E001",
      evidence_references: [],
      confidence: 0.9,
      updated_at: "",
    }]);

    expect(presentation.map(item => item.label)).toEqual([
      "自动化测试：未配置",
      "代码审查：通过",
      "验收证据：充分",
    ]);
    expect(presentation.some(item => item.tone === "error")).toBe(false);
  });
});
