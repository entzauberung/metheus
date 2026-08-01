/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, RecoveryPresentation } from "./types";
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

  it("shows blocking facts but delegates the only recovery action to the top bar", () => {
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
          onInStop={noop}
          onEdStop={noop}
          onSyncProject={noop}
        />,
      );
    });

    expect(host.textContent).toContain("后台执行已阻断");
    expect(host.textContent).toContain("页面顶部的唯一恢复入口");
    expect(host.textContent).not.toContain("预览并恢复执行基线");
    expect(host.querySelectorAll("button")).toHaveLength(0);
  });
});
