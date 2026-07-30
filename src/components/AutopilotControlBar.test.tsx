/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GitConfirmationFailureKind, Project, RecoveryErrorKind, RecoveryPhase, VerificationStage } from "../types";
import { AutopilotControlBar } from "./AutopilotControlBar";

function blockedProject(failureKind: GitConfirmationFailureKind): Project {
  return {
    name: "git-confirmation-test",
    milestones: [],
    workflow_state: {
      top_level_phase: "Console",
      current_step: "Execution",
      autopilot_active: true,
      autopilot_target_milestone_id: "milestone-1",
      autopilot_state: {
        active: true,
        target_milestone_id: "milestone-1",
        run_status: "ErrorStopped",
        last_action: "Git 确认受阻",
        last_action_at: "2026-07-25T00:00:00Z",
        error_message: "confirmation blocked",
        recovery_action: failureKind === "LegacyV1TagConflict"
          ? "RetryGitConfirmation"
          : "WaitHumanDecision",
      },
    },
    execution_session: {
      execution_id: "execution-1",
      active: false,
      milestone_id: "milestone-1",
      mid_stage_id: "mid-1",
      subtask_id: "subtask-1",
      subtask_title: "测试小阶段",
      status: "confirmation_blocked",
      base_commit: "abc123",
      failure_message: "immutable tag conflict",
      confirmation_failure_kind: failureKind,
      started_at: "2026-07-25T00:00:00Z",
      state_entered_at: "2026-07-25T00:00:00Z",
      plan_revision: 1,
      subtask_index: 0,
      total_subtasks: 1,
    },
  } as unknown as Project;
}

function validationProject(
  errorKind: RecoveryErrorKind,
  phase: RecoveryPhase,
  options: {
    verificationStage?: VerificationStage;
    validationRetryCount?: number;
    maxValidationRetries?: number;
    nextValidationRetryAt?: string;
    heartbeatAt?: string;
    runStatus?: "Running" | "Paused" | "ErrorStopped";
  } = {},
): Project {
  const runStatus = options.runStatus ?? (phase === "WaitingHuman" ? "ErrorStopped" : "Running");
  return {
    name: "validation-recovery-test",
    milestones: [],
    workflow_state: {
      top_level_phase: "Console",
      current_step: "Execution",
      autopilot_active: true,
      autopilot_target_milestone_id: "milestone-1",
      autopilot_state: {
        active: true,
        target_milestone_id: "milestone-1",
        run_status: runStatus,
        last_action: "验证中",
        last_action_at: "2026-07-26T00:00:00Z",
        error_message: "validation blocked",
        recovery_action: phase === "WaitingHuman" ? "WaitHumanDecision" : "RunAutomaticRecovery",
        heartbeat_at: options.heartbeatAt,
      },
      recovery_state: {
        error_kind: errorKind,
        phase,
        validation_retry_count: options.validationRetryCount ?? 0,
        max_validation_retries: options.maxValidationRetries ?? 3,
        next_validation_retry_at: options.nextValidationRetryAt,
      },
    },
    execution_session: {
      execution_id: "execution-1",
      active: phase !== "WaitingHuman",
      milestone_id: "milestone-1",
      mid_stage_id: "mid-1",
      subtask_id: "subtask-1",
      subtask_title: "测试小阶段",
      status: "awaiting_confirmation",
      verification_stage: options.verificationStage,
      base_commit: "abc123",
      started_at: "2026-07-26T00:00:00Z",
      state_entered_at: "2026-07-26T00:00:00Z",
      plan_revision: 1,
      subtask_index: 0,
      total_subtasks: 1,
    },
  } as unknown as Project;
}

describe("AutopilotControlBar Git confirmation recovery", () => {
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

  function render(failureKind: GitConfirmationFailureKind) {
    const noop = vi.fn(async () => undefined);
    act(() => {
      root.render(
        <AutopilotControlBar
          project={blockedProject(failureKind)}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
          onAcknowledgeRecovery={noop}
          onRetryCurrent={noop}
          onRetryGitConfirmation={noop}
        />,
      );
    });
    return [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
  }

  it("offers only confirmation retry for a legacy V1 collision", () => {
    const buttons = render("LegacyV1TagConflict");

    expect(host.textContent).toContain("Git 确认受阻");
    expect(host.textContent).toContain("代码与质量结果已保留");
    expect(buttons).toContain("重新确认提交");
    expect(buttons).not.toContain("恢复基线并继续");
    expect(buttons).not.toContain("恢复基线并重试");
  });

  it("requires manual review for a V2 integrity conflict", () => {
    const buttons = render("V2TagIntegrityConflict");

    expect(host.textContent).toContain("请人工核对 V2 不可变标签与确认提交");
    expect(buttons).not.toContain("重新确认提交");
    expect(buttons).not.toContain("恢复基线并继续");
    expect(buttons).not.toContain("恢复基线并重试");
  });

  it("shows backend action and heartbeat without human recovery during automatic retry", () => {
    const project = blockedProject("CommitFailed");
    project.execution_session!.status = "execution_failed";
    project.workflow_state.recovery_state = {
      phase: "WaitingEngine",
      error_kind: "EngineBlocked",
    } as Project["workflow_state"]["recovery_state"];
    Object.assign(project.workflow_state.autopilot_state!, {
      run_status: "Running",
      recovery_action: "RestoreExecutionBaseline",
      current_action_kind: "execute_current_subtask",
      heartbeat_at: "2026-07-25T00:00:10Z",
      transient_retry_count: 2,
      next_retry_at: "2026-07-25T00:00:30Z",
      last_failure_kind: "ProviderUnavailable",
    });
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <AutopilotControlBar
          project={project}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
          onAcknowledgeRecovery={noop}
          onRetryCurrent={noop}
          onResolveHumanRecovery={noop}
        />,
      );
    });

    const text = host.textContent ?? "";
    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(text).toContain("等待自动重试");
    expect(text).toContain("当前：执行当前任务");
    expect(text).toContain("重试 2/3");
    expect(text).toContain("心跳");
    expect(buttons).toContain("暂停自动驾驶");
    expect(buttons).not.toContain("检查引擎并重试");
    expect(buttons).not.toContain("恢复基线并继续");
    expect(buttons).not.toContain("恢复基线并重试");
  });

  it("keeps review service blocks at the settings and revalidation boundary", () => {
    const project = validationProject("ReviewServiceBlocked", "WaitingHuman");
    const onResolveHumanRecovery = vi.fn(async () => undefined);
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <AutopilotControlBar
          project={project}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
          onResolveHumanRecovery={onResolveHumanRecovery}
        />,
      );
    });

    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(host.textContent).toContain("AI 审查认证或额度异常");
    expect(buttons).toContain("打开决策模型设置");
    expect(buttons).toContain("重新验证");
    expect(buttons).not.toContain("恢复基线并继续");
    expect(buttons).not.toContain("恢复基线并重试");
    expect(buttons).not.toContain("手动修复后复测");
    expect(buttons).not.toContain("重新规划当前任务");

    act(() => {
      host.querySelector("button[title='打开决策模型设置']")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onResolveHumanRecovery).not.toHaveBeenCalled();
  });

  it("reports protocol exhaustion without exposing code recovery controls", () => {
    const project = validationProject("ReviewProtocolFailure", "WaitingHuman");
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <AutopilotControlBar
          project={project}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
          onResolveHumanRecovery={noop}
        />,
      );
    });

    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(host.textContent).toContain("审查结果格式持续异常");
    expect(buttons).toContain("重新验证");
    expect(buttons).not.toContain("恢复基线并继续");
    expect(buttons).not.toContain("恢复基线并重试");
    expect(buttons).not.toContain("手动修复后复测");
    expect(buttons).not.toContain("重新规划当前任务");
  });

  it("shows only validation controls while a review retry is pending", () => {
    const project = validationProject("ReviewTransientFailure", "Retesting", {
      verificationStage: "ReviewRetry",
      validationRetryCount: 1,
      maxValidationRetries: 3,
      nextValidationRetryAt: "2026-07-26T00:00:30Z",
    });
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <AutopilotControlBar
          project={project}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
          onAcknowledgeRecovery={noop}
          onRetryCurrent={noop}
          onResolveHumanRecovery={noop}
        />,
      );
    });

    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(host.textContent).toContain("等待验证重试");
    expect(host.textContent).toContain("审查重试 1/3");
    expect(host.textContent).toContain("验证阶段：重新请求 AI 审查");
    expect(buttons).toContain("同步");
    expect(buttons).toContain("暂停验证");
    expect(buttons).not.toContain("恢复基线并继续");
    expect(buttons).not.toContain("恢复基线并重试");
    expect(buttons).not.toContain("检查引擎并重试");
    expect(buttons).not.toContain("重新规划当前任务");
  });

  it("flags a stale heartbeat during an active validation stage", () => {
    const project = validationProject("ReviewTransientFailure", "Retesting", {
      verificationStage: "ParsingReview",
      validationRetryCount: 0,
      maxValidationRetries: 0,
      heartbeatAt: "2026-07-25T23:59:00Z",
    });
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <AutopilotControlBar
          project={project}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
        />,
      );
    });

    expect(host.textContent).toContain("心跳异常");
  });

  it("uses the same deep recovery leaf for quality and evidence status", () => {
    const project = validationProject("EvidenceInsufficient", "WaitingHuman");
    project.workflow_state.recovery_state!.subtask_id = "deep-leaf";
    project.milestones = [{
      id: "milestone-1",
      subtasks: [],
      mid_stages: [{
        id: "mid-1",
        subtasks: [{
          id: "parent",
          status: "Pending",
          child_tasks: [{
            id: "child-parent",
            status: "Pending",
            child_tasks: [{
              id: "deep-leaf",
              status: "AwaitingConfirmation",
              child_tasks: [],
              test_result: {
                passed: false,
                issues: [],
                suggestion: "",
                automated_test_status: "Passed",
              },
              acceptance_ledger: [{
                criterion_index: 1,
                criterion: "深层证据",
                status: "Unknown",
                evidence: "",
                evidence_references: [],
                confidence: 0,
                updated_at: "",
              }],
            }],
          }],
        }],
      }],
    }] as unknown as Project["milestones"];
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <AutopilotControlBar
          project={project}
          executionStatus={null}
          busy={false}
          onToggle={noop}
          onStopManagedFlow={noop}
          onPauseNow={noop}
          onPauseAfterCurrent={noop}
          onResume={noop}
          onSync={noop}
          onResolveHumanRecovery={noop}
        />,
      );
    });

    expect(host.textContent).toContain("自动化测试：通过");
    expect(host.textContent).toContain("验收证据：不足");
  });
});
