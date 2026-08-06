import type { MidStage, Milestone, Project, StagePlanCheckResult, Subtask } from "./types";

export type PlanTargetKind = "Milestone" | "MidStage";

export interface PlanTarget {
  kind: PlanTargetKind;
  milestone: Milestone;
  midStage: MidStage | null;
  title: string;
  version: string;
  subtasks: Subtask[];
  planCheckResult: StagePlanCheckResult | null;
  planApprovedAt: string | null;
  planRevision: number;
  planDraftRevision: number;
  planGeneratedAt: string | null;
  planRegenerationCount: number;
  lastPlanFailureFingerprint: string;
  lastPlanIssueCount: number;
  planNoProgressCount: number;
}

export function resolvePlanTarget(project: Project): PlanTarget | null {
  const profile = project.workload_profile;
  const milestone = project.milestones.find(item => item.id === project.current_milestone_id);
  if (!profile || !milestone) return null;

  if (!profile.use_mid_stage_layer) {
    if (milestone.mode !== "Quick"
      || project.current_mid_stage_id !== ""
      || milestone.mid_stages.length > 0) return null;
    return {
      kind: "Milestone",
      milestone,
      midStage: null,
      title: milestone.title,
      version: milestone.version,
      subtasks: milestone.subtasks,
      planCheckResult: milestone.plan_check_result ?? null,
      planApprovedAt: milestone.plan_approved_at ?? null,
      planRevision: milestone.plan_revision,
      planDraftRevision: milestone.plan_draft_revision,
      planGeneratedAt: milestone.plan_generated_at ?? null,
      planRegenerationCount: milestone.plan_regeneration_count,
      lastPlanFailureFingerprint: milestone.last_plan_failure_fingerprint,
      lastPlanIssueCount: milestone.last_plan_issue_count,
      planNoProgressCount: milestone.plan_no_progress_count,
    };
  }

  if (milestone.mode !== "Professional"
    || project.current_mid_stage_id === ""
    || milestone.subtasks.length > 0) return null;
  const midStage = milestone.mid_stages.find(item => item.id === project.current_mid_stage_id);
  if (!midStage) return null;
  return {
    kind: "MidStage",
    milestone,
    midStage,
    title: midStage.title,
    version: midStage.version,
    subtasks: midStage.subtasks,
    planCheckResult: midStage.plan_check_result ?? null,
    planApprovedAt: midStage.plan_approved_at ?? null,
    planRevision: midStage.plan_revision,
    planDraftRevision: midStage.plan_draft_revision,
    planGeneratedAt: midStage.plan_generated_at ?? null,
    planRegenerationCount: midStage.plan_regeneration_count,
    lastPlanFailureFingerprint: midStage.last_plan_failure_fingerprint ?? "",
    lastPlanIssueCount: midStage.last_plan_issue_count ?? 0,
    planNoProgressCount: midStage.plan_no_progress_count ?? 0,
  };
}
