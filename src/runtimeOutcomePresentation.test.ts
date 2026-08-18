import { describe, expect, it } from "vitest";
import {
  formatRuntimeMutationFeedback,
  resolveRuntimeOutcomePresentation,
  runtimeOutcomeFeedbackType,
} from "./runtimeOutcomePresentation";
import type {
  Project,
  RecoveryPresentation,
  ResourceObservationSummary,
  RuntimeSnapshot,
  Subtask,
} from "./types";

function task(overrides: Partial<Subtask> = {}): Subtask {
  return {
    id: "task-1",
    title: "执行任务",
    status: "Pending",
    acceptance_criteria: [],
    acceptance_ledger: [],
    child_tasks: [],
    ...overrides,
  } as Subtask;
}

function project(currentTask: Subtask): Project {
  return {
    name: "runtime-outcome",
    workflow_state: {
      current_step: "Execution",
      managed_flow_state: undefined,
    },
    milestones: [{ id: "milestone-1", subtasks: [currentTask], mid_stages: [] }],
    execution_session: {
      execution_id: "execution-1",
      subtask_id: currentTask.id,
      status: "awaiting_confirmation",
    },
  } as unknown as Project;
}

function snapshot(
  value: Project,
  pipelineState: RuntimeSnapshot["pipeline_state"] = null,
  recovery: Partial<RecoveryPresentation> = {},
  resourceObservation?: ResourceObservationSummary,
): RuntimeSnapshot {
  return {
    project: value,
    pipeline_state: pipelineState,
    recovery_presentation: {
      kind: "None",
      severity: "Info",
      progress_status: "inactive",
      ...recovery,
    } as RecoveryPresentation,
    resource_observation: resourceObservation,
  } as RuntimeSnapshot;
}

describe("runtime outcome presentation", () => {
  it("keeps the UI read-only until a runtime snapshot exists", () => {
    const view = resolveRuntimeOutcomePresentation({ snapshot: null, syncStatus: "idle" });
    expect(view.state).toBe("unknown");
    expect(view.writeAllowed).toBe(false);
    expect(view.summary).not.toContain("成功");
  });

  it("separates active execution from a completion result", () => {
    const current = task({ status: "Executing" });
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(current), {
        status: "Running",
        current_subtask_id: current.id,
      } as RuntimeSnapshot["pipeline_state"]),
    });
    expect(view.state).toBe("executing");
    expect(view.execution).toBe("pending");
    expect(view.statusLabel).toBe("执行中");
  });

  it("does not call a failed quality result complete", () => {
    const current = task({
      status: "AwaitingConfirmation",
      execution_result: {
        success: true,
        output: "",
        error_log: "",
        file_changes: [],
        engine_runtime: "BuiltIn",
        engine_settings_revision: 0,
        engine_source_revision: "",
        engine_api_backend: "",
        stdout: "",
        stderr: "",
      },
      test_result: { passed: false, issues: [], suggestion: "" },
      acceptance_criteria: ["页面可用"],
      acceptance_ledger: [
        {
          criterion_index: 1,
          criterion: "页面可用",
          status: "Unsatisfied",
          evidence: "",
          evidence_references: [],
          confidence: 0,
          updated_at: "",
        },
      ],
    });
    const view = resolveRuntimeOutcomePresentation({ snapshot: snapshot(project(current)) });
    expect(view.state).toBe("quality_blocked");
    expect(view.state).not.toBe("completed");
    expect(view.confirmation).toBe("required");
  });

  it("requires execution, quality, ledger, and confirmation facts for completion", () => {
    const current = task({
      status: "Passed",
      execution_result: {
        success: true,
        output: "",
        error_log: "",
        file_changes: [],
        engine_runtime: "BuiltIn",
        engine_settings_revision: 0,
        engine_source_revision: "",
        engine_api_backend: "",
        stdout: "",
        stderr: "",
      },
      test_result: { passed: true, issues: [], suggestion: "" },
      acceptance_criteria: ["页面可用"],
      acceptance_ledger: [
        {
          criterion_index: 1,
          criterion: "页面可用",
          status: "Satisfied",
          evidence: "",
          evidence_references: [],
          confidence: 1,
          updated_at: "",
        },
      ],
    });
    const view = resolveRuntimeOutcomePresentation({ snapshot: snapshot(project(current)) });
    expect(view.state).toBe("completed");
    expect(view.execution).toBe("passed");
    expect(view.quality).toBe("passed");
    expect(view.acceptance).toBe("passed");
    expect(view.confirmation).toBe("confirmed");
  });

  it("does not accept duplicate or mismatched ledger rows as complete coverage", () => {
    const current = task({
      status: "Passed",
      execution_result: {
        success: true,
        output: "",
        error_log: "",
        file_changes: [],
        engine_runtime: "BuiltIn",
        engine_settings_revision: 0,
        engine_source_revision: "",
        engine_api_backend: "",
        stdout: "",
        stderr: "",
      },
      test_result: { passed: true, issues: [], suggestion: "" },
      acceptance_criteria: ["first", "second"],
      acceptance_ledger: [
        {
          criterion_index: 1,
          criterion: "first",
          status: "Satisfied",
          evidence: "one",
          evidence_references: [],
          confidence: 1,
          updated_at: "",
        },
        {
          criterion_index: 1,
          criterion: "first",
          status: "Satisfied",
          evidence: "duplicate",
          evidence_references: [],
          confidence: 1,
          updated_at: "",
        },
      ],
    });
    const view = resolveRuntimeOutcomePresentation({ snapshot: snapshot(project(current)) });
    expect(view.acceptance).toBe("blocked");
    expect(view.state).not.toBe("completed");
  });

  it("does not let an inactive recovery presentation hide a completed result", () => {
    const current = task({
      status: "Passed",
      execution_result: {
        success: true,
        output: "",
        error_log: "",
        file_changes: [],
        engine_runtime: "BuiltIn",
        engine_settings_revision: 0,
        engine_source_revision: "",
        engine_api_backend: "",
        stdout: "",
        stderr: "",
      },
      test_result: { passed: true, issues: [], suggestion: "" },
      acceptance_criteria: ["页面可用"],
      acceptance_ledger: [{
        criterion_index: 1,
        criterion: "页面可用",
        status: "Satisfied",
        evidence: "",
        evidence_references: [],
        confidence: 1,
        updated_at: "",
      }],
    });
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(current), null, {
        kind: "AutomaticRecovery",
        severity: "Info",
        progress_status: "inactive",
      }),
    });
    expect(view.state).toBe("completed");
    expect(view.recoveryKind).toBe("AutomaticRecovery");
  });

  it("prioritizes a failed session over an inconsistent running pipeline", () => {
    const current = task({ status: "Executing" });
    const value = project(current);
    value.execution_session!.status = "execution_failed";
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(value, {
        status: "Running",
        current_subtask_id: current.id,
      } as RuntimeSnapshot["pipeline_state"]),
    });
    expect(view.execution).toBe("blocked");
    expect(view.state).toBe("failed");
    expect(view.tone).toBe("error");
  });

  it("maps an active backend recovery to a separate recovery state", () => {
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(task()), null, {
        kind: "AutomaticRecovery",
        severity: "Warning",
        progress_status: "running",
      }),
    });
    expect(view.state).toBe("recovering");
    expect(view.recoveryKind).toBe("AutomaticRecovery");
  });

  it("does not claim background recovery when progress is missing", () => {
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(task()), null, {
        kind: "AutomaticRecovery",
        severity: "Warning",
      }),
    });
    expect(view.state).toBe("waiting_human");
    expect(view.tone).toBe("error");
  });

  it("preserves facts but blocks writes while runtime sync is stale", () => {
    const current = task({ status: "Executing" });
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(current)),
      syncStatus: "disconnected",
    });
    expect(view.state).toBe("executing");
    expect(view.syncFresh).toBe(false);
    expect(view.writeAllowed).toBe(false);
    expect(view.writeBlockedReason).toContain("断开");
  });

  it("does not treat a completed workflow marker as completion without facts", () => {
    const current = task({ status: "Pending" });
    const completedProject = project(current);
    completedProject.workflow_state.current_step = "Completed";
    const view = resolveRuntimeOutcomePresentation({ snapshot: snapshot(completedProject) });
    expect(view.state).toBe("unknown");
    expect(view.state).not.toBe("completed");
  });

  it("does not treat a completed pipeline as success without an execution result", () => {
    const current = task({ status: "Pending" });
    const view = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(current), {
        status: "Completed",
        current_subtask_id: current.id,
      } as RuntimeSnapshot["pipeline_state"]),
    });
    expect(view.state).toBe("unknown");
    expect(view.state).not.toBe("completed");
  });

  it("keeps mutation actions separate from the current task outcome", () => {
    const current = task({ status: "Executing" });
    const view = resolveRuntimeOutcomePresentation({ snapshot: snapshot(project(current)) });
    expect(formatRuntimeMutationFeedback("自动驾驶 owner 已恢复", view))
      .toContain("自动驾驶 owner 已恢复；当前任务：执行中");
    expect(runtimeOutcomeFeedbackType(view)).toBe("info");

    expect(formatRuntimeMutationFeedback("", view)).toContain("后端未提供动作摘要");
    expect(formatRuntimeMutationFeedback("", view)).not.toContain("任务已完成");
  });

  it("shows resource provenance and refuses to call an unknown sample safe", () => {
    const unknownView = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(task())),
    });
    expect(unknownView.summary).toContain("资源状态：未知（不能视为安全）");
    expect(unknownView.summary).toContain("来源：未知");

    const measuredView = resolveRuntimeOutcomePresentation({
      snapshot: snapshot(project(task()), null, {}, {
        state: "HardStop",
        source: "Cgroup",
        headroom_bytes: 1024,
        sampled_at: "2026-08-18T00:00:00Z",
      }),
    });
    expect(measuredView.summary).toContain("资源状态：硬停止");
    expect(measuredView.summary).toContain("来源：cgroup");
    expect(measuredView.summary).toContain("余量：1.0 KiB");
    expect(measuredView.summary).toContain("采样：2026-08-18T00:00:00Z");
  });
});
