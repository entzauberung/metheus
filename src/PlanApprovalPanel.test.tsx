/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "./types";
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

  function render(project: Project) {
    act(() => {
      root.render(
        <PlanApprovalPanel
          project={project}
          onReturnToDiscussion={vi.fn()}
          onApprove={vi.fn()}
          onReject={vi.fn()}
          onEnterConsole={vi.fn()}
          isSubmitting={false}
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
});
