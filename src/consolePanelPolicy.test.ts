import { describe, expect, it } from "vitest";
import { shouldCollapseConsolePanel } from "./consolePanelPolicy";

describe("console panel policy", () => {
  it("collapses when entering MilestoneReview", () => {
    expect(shouldCollapseConsolePanel("Execution", "MilestoneReview")).toBe(true);
    expect(shouldCollapseConsolePanel(null, "MilestoneReview")).toBe(true);
  });

  it("does not collapse again while MilestoneReview remains active", () => {
    expect(shouldCollapseConsolePanel("MilestoneReview", "MilestoneReview")).toBe(false);
  });

  it("does not collapse for unrelated transitions", () => {
    expect(shouldCollapseConsolePanel("Execution", "PauseDecision")).toBe(false);
    expect(shouldCollapseConsolePanel("MilestoneReview", "BranchDiscussion")).toBe(false);
    expect(shouldCollapseConsolePanel(null, null)).toBe(false);
  });

  it("collapses again after leaving and re-entering MilestoneReview", () => {
    expect(shouldCollapseConsolePanel("BranchDiscussion", "MilestoneReview")).toBe(true);
  });
});
