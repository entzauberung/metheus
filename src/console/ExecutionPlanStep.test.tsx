/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, Subtask } from "../types";
import { ExecutionPlanStep } from "./ExecutionPlanStep";

function task(id: string, title: string): Subtask {
  return {
    id,
    title,
    order: 1,
    status: "Pending",
    goal: title,
    context_summary: "计划上下文",
    allowed_file_paths: ["src/index.ts"],
    new_file_paths: [],
    acceptance_criteria: ["交付可验证"],
    stop_rules: ["越界即停止"],
    child_tasks: [],
  } as unknown as Subtask;
}

function quickProject(): Project {
  return {
    current_milestone_id: "milestone-1",
    current_mid_stage_id: "",
    workload_profile: { use_mid_stage_layer: false },
    workflow_state: { current_step: "PlanCheck" },
    milestones: [{
      id: "milestone-1",
      title: "静态网页",
      version: "v0.1",
      mode: "Quick",
      status: "Pending",
      mid_stages: [],
      subtasks: [task("direct-task", "实现静态页面")],
      plan_revision: 0,
      plan_draft_revision: 1,
      plan_regeneration_count: 0,
    }],
  } as unknown as Project;
}

function professionalProject(): Project {
  return {
    current_milestone_id: "milestone-1",
    current_mid_stage_id: "mid-1",
    workload_profile: { use_mid_stage_layer: true },
    workflow_state: { current_step: "PlanCheck" },
    milestones: [{
      id: "milestone-1",
      title: "全栈系统",
      version: "v0.1",
      mode: "Professional",
      status: "InProgress",
      subtasks: [],
      mid_stages: [{
        id: "mid-1",
        title: "权限后端",
        version: "v0.1.1",
        status: "Ready",
        subtasks: [task("mid-task", "实现权限接口")],
        plan_revision: 0,
        plan_draft_revision: 1,
        plan_regeneration_count: 0,
      }],
    }],
  } as unknown as Project;
}

describe("ExecutionPlanStep plan targets", () => {
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
    const noop = vi.fn();
    act(() => {
      root.render(
        <ExecutionPlanStep
          project={project}
          busy={false}
          feedback={null}
          regenerationFeedback=""
          setRegenerationFeedback={noop}
          regenerationModalOpen={false}
          setRegenerationModalOpen={noop}
          onGenerate={noop}
          onCheck={noop}
          onApprove={noop}
          onRegenerate={noop}
          workspaceStatus={null}
          onPrepareWorkspace={vi.fn(async () => undefined)}
        />,
      );
    });
  }

  it("reads a Quick plan directly from the milestone", () => {
    render(quickProject());
    expect(host.textContent).toContain("实现静态页面");
    expect(host.textContent).not.toContain("计划目标不可用");
    expect(host.textContent).not.toContain("权限后端");
  });

  it("keeps the Professional plan on the selected mid-stage", () => {
    render(professionalProject());
    expect(host.textContent).toContain("实现权限接口");
    expect(host.textContent).not.toContain("计划目标不可用");
  });
});
