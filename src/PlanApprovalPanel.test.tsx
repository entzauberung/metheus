/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ManagedFlowState, Project } from "./types";
import { PlanApprovalPanel } from "./PlanApprovalPanel";

function projectWithProfile(): Project {
  return {
    name: "profile-approval",
    discussion_revision: 3,
    entry_kind: "NoProject",
    workload_profile: {
      signals: {
        has_frontend: true,
        has_backend: false,
        has_persistence: false,
        has_auth_or_roles: false,
        external_integration_count: 0,
        independent_domain_count: 1,
        deliverable_count: 2,
        high_risk: false,
      },
      scale: "Small",
      use_mid_stage_layer: false,
      max_milestones: 1,
      max_mid_stages: 0,
      max_subtasks: 3,
      max_split_depth: 0,
      check_depth: "Lean",
      max_executor_turns: 8,
      max_transport_retries: 1,
      max_doom_loop_retries: 0,
      evidence: ["范围事实：1 个独立领域，2 个交付物"],
      discussion_revision: 3,
      fingerprint: "sha256:profile",
    },
    plan_draft: {
      draft_id: "draft-1",
      draft_status: "Pending",
      plan_content: "计划正文",
      constitution_part1_draft: "宪法正文",
      generation_revision: 3,
      data_revision_at_generation: 4,
      workload_profile_fingerprint: "sha256:profile",
      self_check_result: "",
      generated_at: "2026-08-06T00:00:00Z",
      approved: false,
    },
    workflow_state: {},
  } as unknown as Project;
}

function errorStoppedManaged(): ManagedFlowState {
  return {
    active: true,
    managed_state: "ErrorStopped",
    managed_target: "MilestoneSelection",
    last_action: "模型连接失败",
    last_action_at: "2026-08-13T00:00:00Z",
    run_status: "ErrorStopped",
    error_message: "provider unavailable",
    job_id: "job-1",
    job_generation: 2,
    current_action: "",
    current_action_id: "",
    heartbeat_at: "2026-08-13T00:00:00Z",
    retry_count: 0,
    last_completed_action: "generate_version_plan",
  };
}

describe("PlanApprovalPanel workload binding", () => {
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

  function render(
    project: Project,
    extras?: {
      managedFlowState?: ManagedFlowState;
      onResumeManagedFlow?: () => void;
      onStopManagedFlow?: () => void;
      onStartManagedFlow?: () => void;
      onPauseManagedFlow?: () => void;
    },
  ) {
    act(() => {
      root.render(
        <PlanApprovalPanel
          project={project}
          onReturnToDiscussion={vi.fn()}
          onApprove={vi.fn()}
          onReject={vi.fn()}
          onEnterConsole={vi.fn()}
          isSubmitting={false}
          managedFlowState={extras?.managedFlowState}
          onResumeManagedFlow={extras?.onResumeManagedFlow}
          onStopManagedFlow={extras?.onStopManagedFlow}
          onStartManagedFlow={extras?.onStartManagedFlow}
          onPauseManagedFlow={extras?.onPauseManagedFlow}
        />,
      );
    });
  }

  it("shows the backend scale, topology, limits, and evidence", () => {
    render(projectWithProfile());
    expect(host.textContent).toContain("工作负载画像：Small");
    expect(host.textContent).toContain("Milestone → Subtask");
    expect(host.textContent).toContain("Subtask 3");
    expect(host.textContent).toContain("1 个独立领域，2 个交付物");
    const approve = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("批准项目方案"));
    expect(approve?.disabled).toBe(false);
  });

  it("blocks approval when the profile is missing or its fingerprint changed", () => {
    const missing = projectWithProfile();
    missing.workload_profile = undefined;
    render(missing);
    expect(host.textContent).toContain("工作负载画像缺失");
    let approve = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("批准项目方案"));
    expect(approve?.disabled).toBe(true);

    const changed = projectWithProfile();
    changed.workload_profile!.fingerprint = "sha256:changed";
    act(() => root.render(
      <PlanApprovalPanel
        project={changed}
        onReturnToDiscussion={vi.fn()}
        onApprove={vi.fn()}
        onReject={vi.fn()}
        onEnterConsole={vi.fn()}
        isSubmitting={false}
      />,
    ));
    expect(host.textContent).toContain("指纹不一致");
    approve = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("批准项目方案"));
    expect(approve?.disabled).toBe(true);
  });

  it("keeps ErrorStopped managed controls with restart and stop handlers", () => {
    const onResume = vi.fn();
    const onStop = vi.fn();
    const project = projectWithProfile();
    project.workflow_state = {
      managed_flow_state: errorStoppedManaged(),
    } as Project["workflow_state"];

    render(project, {
      managedFlowState: errorStoppedManaged(),
      onResumeManagedFlow: onResume,
      onStopManagedFlow: onStop,
    });

    expect(host.textContent).toContain("托管层因错误停止");
    expect(host.textContent).toContain("provider unavailable");
    expect(host.textContent).toContain("重新启动托管");
    expect(host.textContent).toContain("停止托管并转人工");
    expect(host.querySelector("[data-testid=\"plan-approval-managed-controls\"]")).not.toBeNull();

    const restart = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("重新启动托管"));
    const stop = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("停止托管并转人工"));
    expect(restart).toBeTruthy();
    expect(stop).toBeTruthy();

    act(() => {
      restart?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      stop?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onResume).toHaveBeenCalledTimes(1);
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("shows ErrorStopped controls on approved plan view", () => {
    const project = projectWithProfile();
    project.plan_draft!.draft_status = "Approved";
    project.plan_draft!.approved = true;
    project.plan_draft!.approved_at = "2026-08-13T01:00:00Z";
    project.workflow_state = {
      managed_flow_state: errorStoppedManaged(),
    } as Project["workflow_state"];

    render(project, {
      managedFlowState: errorStoppedManaged(),
      onResumeManagedFlow: vi.fn(),
      onStopManagedFlow: vi.fn(),
    });

    expect(host.textContent).toContain("项目方案已批准");
    expect(host.textContent).toContain("重新启动托管");
    expect(host.textContent).toContain("停止托管并转人工");
  });
});
