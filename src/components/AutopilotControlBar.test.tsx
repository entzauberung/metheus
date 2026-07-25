/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GitConfirmationFailureKind, Project } from "../types";
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
});
