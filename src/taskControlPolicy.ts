import type { AcceptanceLedgerItem, CostGroupSummary, TaskControlMode, TaskTreeNodeView, TokenCostSummary } from "./types";

export function getTaskControlModeLabel(mode: TaskControlMode): string {
  return { Legacy: "旧流水线", Shadow: "影子控制器", SerialTakeover: "串行接管" }[mode];
}

export function countTaskNodes(nodes: TaskTreeNodeView[]): number {
  return nodes.reduce((total, node) => total + 1 + countTaskNodes(node.children), 0);
}

export function acceptanceCounts(items: AcceptanceLedgerItem[]) {
  return items.reduce((counts, item) => {
    counts[item.status] += 1;
    return counts;
  }, { Satisfied: 0, Unsatisfied: 0, Unknown: 0, Contradictory: 0, AcceptedDeviation: 0 });
}

export function hasControlCapability(capabilities: string[], action: string): boolean {
  return capabilities.includes(action);
}

export function tokenUsageCoverage(summary: TokenCostSummary) {
  const knownCalls = summary.usage_known_calls ?? (summary.total_tokens === undefined ? 0 : summary.calls);
  const unknownCalls = summary.usage_unknown_calls ?? Math.max(0, summary.calls - knownCalls);
  const knownTotal = knownCalls > 0
    ? (summary.known_total_tokens ?? summary.total_tokens)
    : undefined;
  return {
    knownCalls,
    unknownCalls,
    knownTotal,
    percentage: summary.calls === 0 ? 0 : Math.round((knownCalls / summary.calls) * 100),
  };
}

export function visibleCostGroups(groups: CostGroupSummary[], limit = 4): CostGroupSummary[] {
  return [...groups]
    .sort((left, right) => right.summary.calls - left.summary.calls || left.key.localeCompare(right.key))
    .slice(0, limit);
}
