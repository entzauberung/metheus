import type { WorkflowStep } from "./types";

export function shouldCollapseConsolePanel(
  previousStep: WorkflowStep | null,
  nextStep: WorkflowStep | null,
): boolean {
  return nextStep === "MilestoneReview" && previousStep !== "MilestoneReview";
}
