import { describe, expect, it } from "vitest";
import { acceptanceCounts, countTaskNodes, getModeTransitionImpact, getTaskControlModeDescription, getTaskControlModeLabel, hasControlCapability, requiresModeFallbackConfirmation, tokenUsageCoverage, visibleCostGroups } from "./taskControlPolicy";
import type { TokenCostSummary } from "./types";

const cost = (overrides: Partial<TokenCostSummary> = {}): TokenCostSummary => ({
  calls: 0,
  known_input_tokens: 0,
  known_output_tokens: 0,
  known_total_tokens: 0,
  usage_known_calls: 0,
  usage_unknown_calls: 0,
  effective_calls: 0,
  no_progress_calls: 0,
  ...overrides,
});

describe("task control presentation policy", () => {
  it("keeps backend mode names as display-only labels", () => {
    expect(getTaskControlModeLabel("SerialTakeover")).toContain("新项目默认");
    expect(getTaskControlModeLabel("Shadow")).toContain("仅审计");
    expect(getTaskControlModeLabel("Legacy")).toContain("兼容");
    expect(getTaskControlModeDescription("Shadow")).toContain("实际执行仍由旧流水线负责");
  });

  it("requires an explicit reasoned confirmation only when leaving serial takeover", () => {
    expect(requiresModeFallbackConfirmation("SerialTakeover", "Shadow")).toBe(true);
    expect(requiresModeFallbackConfirmation("SerialTakeover", "Legacy")).toBe(true);
    expect(requiresModeFallbackConfirmation("Shadow", "SerialTakeover")).toBe(false);
    expect(getModeTransitionImpact("Shadow").join(" ")).toContain("任务树");
    expect(getModeTransitionImpact("Shadow").join(" ")).toContain("不再派发");
  });

  it("counts arbitrary-depth nodes without changing their state", () => {
    const leaf = {
      id: "t",
      title: "t",
      node_type: "Subtask" as const,
      status: "Pending",
      depth: 1,
      complexity: "Small",
      risk: "Low",
      contract_fingerprint: "",
      dependencies: [],
      acceptance: [],
      capabilities: [],
      disabled_reasons: {},
      is_currently_actionable: false,
      actionable_acceptance_criteria: [],
      children: [],
    };
    expect(countTaskNodes([{
      ...leaf,
      id: "m",
      title: "m",
      node_type: "Milestone",
      depth: 0,
      complexity: "stage",
      risk: "stage",
      children: [leaf],
    }])).toBe(2);
  });

  it("preserves the distinction between unknown and unsatisfied evidence", () => {
    expect(acceptanceCounts([
      { criterion_index: 1, criterion: "a", status: "Unknown", evidence: "", evidence_references: [], confidence: 0, updated_at: "" },
      { criterion_index: 2, criterion: "b", status: "Unsatisfied", evidence: "", evidence_references: [], confidence: 0, updated_at: "" },
    ])).toEqual({
      Satisfied: 0,
      Unsatisfied: 1,
      Unknown: 1,
      Contradictory: 0,
      AcceptedDeviation: 0,
      AiProvisionallySatisfied: 0,
      DeferredHumanReview: 0,
    });
  });

  it("uses only backend capabilities to enable control commands", () => {
    const capabilities = ["pause", "revalidate"];
    expect(hasControlCapability(capabilities, "revalidate")).toBe(true);
    expect(hasControlCapability(capabilities, "split")).toBe(false);
  });

  it("shows known token totals without hiding them behind unknown calls", () => {
    expect(tokenUsageCoverage(cost({
      calls: 3,
      known_total_tokens: 120,
      usage_known_calls: 2,
      usage_unknown_calls: 1,
    }))).toEqual({ knownCalls: 2, unknownCalls: 1, knownTotal: 120, percentage: 67 });
    expect(tokenUsageCoverage(cost({ calls: 2, usage_unknown_calls: 2 })).knownTotal).toBeUndefined();
  });

  it("orders backend cost groups by call volume for compact display", () => {
    const groups = visibleCostGroups([
      { key: "Claude Code", summary: cost({ calls: 1 }) },
      { key: "OpenAI Compatible", summary: cost({ calls: 4 }) },
    ]);
    expect(groups.map(group => group.key)).toEqual(["OpenAI Compatible", "Claude Code"]);
  });
});
