import { Coins, Database } from "lucide-react";
import { tokenUsageCoverage, visibleCostGroups } from "./taskControlPolicy";
import type { CostGroupSummary, ModelCallRecord, TokenCostSummary } from "./types";

interface Props {
  cost: TokenCostSummary;
  stageCost?: TokenCostSummary;
  taskCost?: TokenCostSummary;
  calls?: ModelCallRecord[];
  providerCosts?: CostGroupSummary[];
  purposeCosts?: CostGroupSummary[];
}

export default function TaskCostPanel({ cost, stageCost, taskCost, calls = [], providerCosts = [], purposeCosts = [] }: Props) {
  const coverage = tokenUsageCoverage(cost);
  const value = (amount?: number) => amount === undefined ? "未知" : amount.toLocaleString("zh-CN");
  const knownValue = (amount: number) => coverage.knownCalls === 0 ? "未知" : value(amount);
  const groups = (items: CostGroupSummary[]) => visibleCostGroups(items).map(group => {
    const usage = tokenUsageCoverage(group.summary);
    return <li key={group.key}><span>{group.key}</span><strong>{usage.knownTotal === undefined ? "未知" : value(usage.knownTotal)}</strong><small>{group.summary.calls} 次</small></li>;
  });
  return <section className="task-control-panel"><div className="task-control-panel-title"><Coins size={16} /><h3>Token 成本</h3></div>
    <div className="task-cost-grid"><span><strong>{cost.calls}</strong>调用</span><span><strong>{knownValue(cost.known_input_tokens)}</strong>已知输入</span><span><strong>{knownValue(cost.known_output_tokens)}</strong>已知输出</span><span><strong>{value(coverage.knownTotal)}</strong>已知总计</span></div>
    <p className="task-control-muted"><Database size={14} />用量覆盖 {coverage.percentage}% · 未知 {coverage.unknownCalls} 次 · 当前任务 {taskCost?.calls ?? 0} 次 · 当前阶段 {stageCost?.calls ?? 0} 次 · 有效 {cost.effective_calls} · 无进展 {cost.no_progress_calls}</p>
    {(providerCosts.length > 0 || purposeCosts.length > 0) && <div className="task-cost-groups">
      <div><strong>按供应方</strong><ul>{groups(providerCosts)}</ul></div>
      <div><strong>按用途</strong><ul>{groups(purposeCosts)}</ul></div>
    </div>}
    {calls.length > 0 && <ul className="task-cost-calls">{calls.slice(0, 6).map(call => <li key={call.call_id}><strong>{call.purpose ?? "历史调用"}</strong><span>{call.provider || call.model || "历史/未知"}</span><span>{call.elapsed_ms === undefined ? "耗时未知" : `${call.elapsed_ms} ms`}</span><span>{call.cache_hit ? "缓存" : call.no_progress ? "无进展" : call.produced_change || call.produced_evidence || call.produced_plan || call.produced_contract || call.produced_fact ? "有效变化" : "完成"}</span></li>)}</ul>}
  </section>;
}
