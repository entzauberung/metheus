import type { AcceptanceLedgerItem, CostGroupSummary, TaskControlMode, TaskTreeNodeView, TokenCostSummary } from "./types";

export function getTaskControlModeLabel(mode: TaskControlMode): string {
  return {
    Legacy: "旧流水线（兼容）",
    Shadow: "影子控制器（仅审计）",
    SerialTakeover: "串行接管（新项目默认）",
  }[mode];
}

export function getTaskControlModeDescription(mode: TaskControlMode): string {
  return {
    Legacy: "旧流水线拥有执行权，新控制器不参与决策。",
    Shadow: "新控制器只做对照审计，实际执行仍由旧流水线负责。",
    SerialTakeover: "v0.0.4 正式默认：新控制器拥有任务执行阶段的串行派发与恢复决策权。",
  }[mode];
}

export function requiresModeFallbackConfirmation(
  current: TaskControlMode,
  next: TaskControlMode,
): boolean {
  return current === "SerialTakeover" && next !== "SerialTakeover";
}

export function getModeTransitionImpact(next: TaskControlMode): string[] {
  return [
    "现有任务合同、任务树、证据账本和成本账本都会保留。",
    next === "Shadow"
      ? "新控制器只继续记录对照决策，不再派发任务控制动作。"
      : "新控制器停止参与任务级决策，任务执行交还旧流水线。",
    "活动执行、控制动作、自动推进或恢复期间不能切换模式。",
  ];
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
