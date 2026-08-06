import { describe, expect, it } from "vitest";
import { resolvePlanTarget } from "./planTargetPolicy";
import type { MidStage, Milestone, Project, WorkloadProfile } from "./types";

function profile(useMidStageLayer: boolean): WorkloadProfile {
  return {
    signals: {
      has_frontend: true,
      has_backend: useMidStageLayer,
      has_persistence: false,
      has_auth_or_roles: false,
      external_integration_count: 0,
      independent_domain_count: useMidStageLayer ? 3 : 1,
      deliverable_count: useMidStageLayer ? 3 : 1,
      high_risk: false,
    },
    scale: useMidStageLayer ? "Standard" : "Small",
    use_mid_stage_layer: useMidStageLayer,
    max_milestones: useMidStageLayer ? 3 : 1,
    max_mid_stages: useMidStageLayer ? 3 : 0,
    max_subtasks: useMidStageLayer ? 6 : 3,
    max_split_depth: useMidStageLayer ? 1 : 0,
    check_depth: useMidStageLayer ? "Standard" : "Lean",
    max_executor_turns: useMidStageLayer ? 16 : 8,
    max_transport_retries: 1,
    max_doom_loop_retries: 1,
    evidence: [],
    discussion_revision: 1,
    fingerprint: useMidStageLayer ? "professional" : "quick",
  };
}

function milestone(mode: "Quick" | "Professional"): Milestone {
  return {
    id: "milestone-1",
    version: "v0.1",
    title: "Milestone",
    mode,
    status: "Pending",
    mid_stages: [],
    subtasks: [],
    plan_revision: 0,
    plan_draft_revision: 0,
    plan_regeneration_count: 0,
    last_plan_failure_fingerprint: "",
    last_plan_issue_count: 0,
    plan_no_progress_count: 0,
  } as unknown as Milestone;
}

function project(useMidStageLayer: boolean): Project {
  const item = milestone(useMidStageLayer ? "Professional" : "Quick");
  if (useMidStageLayer) {
    item.mid_stages = [{
      id: "mid-1",
      title: "Mid",
      version: "v0.1.1",
      status: "Ready",
      subtasks: [],
      plan_revision: 0,
      plan_draft_revision: 2,
      plan_regeneration_count: 0,
    } as unknown as MidStage];
  }
  return {
    current_milestone_id: "milestone-1",
    current_mid_stage_id: useMidStageLayer ? "mid-1" : "",
    workload_profile: profile(useMidStageLayer),
    milestones: [item],
  } as unknown as Project;
}

describe("resolvePlanTarget", () => {
  it("resolves a Quick milestone as the direct task container", () => {
    const target = resolvePlanTarget(project(false));
    expect(target?.kind).toBe("Milestone");
    expect(target?.midStage).toBeNull();
  });

  it("resolves a Professional selected mid-stage", () => {
    const target = resolvePlanTarget(project(true));
    expect(target?.kind).toBe("MidStage");
    expect(target?.planDraftRevision).toBe(2);
  });

  it("rejects mixed Quick and mid-stage state", () => {
    const source = project(false);
    source.current_mid_stage_id = "mid-1";
    expect(resolvePlanTarget(source)).toBeNull();
  });
});
