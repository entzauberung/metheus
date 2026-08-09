/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ExecutionWorkspaceStatus, Project, RecoveryPresentation } from "./types";
import V1ExecutionPanel from "./V1ExecutionPanel";

describe("V1ExecutionPanel recovery responsibility", () => {
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
  });

  it("keeps recovery details and actions out of the execution panel", () => {
    const project = {
      name: "blocked-project",
      milestones: [],
      current_milestone_id: "",
      current_mid_stage_id: "",
      workflow_state: { recovery_state: {}, current_step: "Execution" },
      execution_session: {
        subtask_id: "task-1",
        subtask_title: "受阻任务",
        base_commit: "abcdef123456",
      },
    } as unknown as Project;
    const recovery: RecoveryPresentation = {
      kind: "BaselineRecovery",
      title: "执行失败",
      reason: "后台执行已阻断",
      severity: "Error",
      primary_action: {
        capability: "AcknowledgeExecutionRecovery",
        label: "预览并恢复执行基线",
        enabled: true,
        disabled_reason: null,
      },
      secondary_actions: [],
      preserve_current_code: false,
      requires_baseline_restore: true,
      supports_preview: true,
      automatic_retry: false,
      capabilities: ["AcknowledgeExecutionRecovery"],
      decision_options: [],
      state_fingerprint: "blocked",
    };
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <V1ExecutionPanel
          project={project}
          executionStatus={null}
          workspaceStatus={null}
          recoveryPresentation={recovery}
          busy={false}
          onPrepareWorkspace={noop}
          onExecute={noop}
          onConfirm={noop}
          onReject={noop}
        />,
      );
    });

    expect(host.textContent).not.toContain("后台执行已阻断");
    expect(host.textContent).not.toContain("页面顶部的唯一恢复入口");
    expect(host.textContent).not.toContain("预览并恢复执行基线");
    expect(host.querySelectorAll("button")).toHaveLength(0);
  });

  it("executes an approved Quick milestone task without a mid-stage container", async () => {
    const project = {
      name: "quick-project",
      current_milestone_id: "milestone-1",
      current_mid_stage_id: "",
      workload_profile: {
        scale: "Small",
        use_mid_stage_layer: false,
        fingerprint: "quick-profile",
      },
      workflow_state: { current_step: "Execution" },
      milestones: [{
        id: "milestone-1",
        title: "静态网页",
        version: "v0.1",
        mode: "Quick",
        status: "InProgress",
        mid_stages: [],
        subtasks: [{
          id: "direct-task",
          title: "实现页面",
          goal: "交付静态页面",
          status: "Pending",
          allowed_file_paths: ["index.html"],
          new_file_paths: [],
          acceptance_criteria: ["页面可打开"],
          child_tasks: [],
        }],
        plan_approved_at: "2026-08-06T00:00:00Z",
        plan_revision: 1,
      }],
    } as unknown as Project;
    const workspace = {
      ready_for_new_execution: true,
      changes: [],
    } as unknown as ExecutionWorkspaceStatus;
    const onExecute = vi.fn(async () => undefined);
    const noop = vi.fn(async () => undefined);

    act(() => {
      root.render(
        <V1ExecutionPanel
          project={project}
          executionStatus={null}
          workspaceStatus={workspace}
          recoveryPresentation={null}
          busy={false}
          onPrepareWorkspace={noop}
          onExecute={onExecute}
          onConfirm={noop}
          onReject={noop}
        />,
      );
    });

    expect(host.textContent).toContain("下一个任务：实现页面");
    expect(host.textContent).not.toContain("当前中阶段");
    const execute = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("执行当前小阶段"));
    await act(async () => execute?.click());
    expect(onExecute).toHaveBeenCalledOnce();
  });
});
