import { describe, expect, it } from "vitest";
import { getManagedFlowPresentation, getMilestoneApprovalPolicy } from "./managedFlowPolicy";
import type { ManagedFlowState, MilestoneDraft } from "./types";

function managed(run_status: ManagedFlowState["run_status"]): ManagedFlowState {
  return {
    active: true,
    managed_state: "MilestoneApproval",
    managed_target: "MilestoneSelection",
    last_action: "等待处理",
    last_action_at: "2026-07-22T00:00:00Z",
    run_status,
    error_message: "",
  };
}

function draft(status: MilestoneDraft["status"], checkResult = "检查通过"): MilestoneDraft {
  return {
    draft_id: "draft-1",
    status,
    draft_kind: "Normal",
    candidate_milestones: [{ id: "milestone-1" } as MilestoneDraft["candidate_milestones"][number]],
    check_result: checkResult,
    generation_revision: 1,
    source_plan_revision: 1,
    generated_at: "2026-07-22T00:00:00Z",
    regeneration_count: 0,
    retained_milestone_ids: [],
    future_candidate_ids: [],
    original_ai_versions: [],
    normalized_versions: [],
    versions_normalized: false,
    count_expansion_warning: false,
    granularity_check_passed: false,
    granularity_issues: [],
  };
}

describe("managed flow presentation", () => {
  it("distinguishes every persisted managed run state", () => {
    expect(getManagedFlowPresentation(managed("Running"), "MilestoneCheck").statusLabel).toBe("托管层运行中");
    expect(getManagedFlowPresentation(managed("Paused"), "MilestoneCheck").statusLabel).toBe("托管层已暂停");
    expect(getManagedFlowPresentation(managed("WaitingHuman"), "MilestoneCheck").statusLabel).toBe("托管层等待人工处理");
    expect(getManagedFlowPresentation(managed("ErrorStopped"), "MilestoneCheck").statusLabel).toBe("托管层因错误停止");
  });

  it("names a resumed checked draft as an approval action", () => {
    expect(getManagedFlowPresentation(
      managed("Paused"),
      "MilestoneApproval",
      draft("CheckPassed"),
    )).toMatchObject({ canResume: true, resumeLabel: "继续托管并批准" });
  });
});

describe("milestone approval policy", () => {
  it("only enables approval for an explicitly passed, non-empty draft", () => {
    expect(getMilestoneApprovalPolicy(draft("CheckPassed")).canApprove).toBe(true);
    expect(getMilestoneApprovalPolicy(draft("Pending")).canApprove).toBe(false);
    expect(getMilestoneApprovalPolicy(draft("CheckFailed")).canApprove).toBe(false);
    expect(getMilestoneApprovalPolicy(draft("CheckPassed", " ")).canApprove).toBe(false);
  });
});
