import { ArrowRight, BrainCircuit } from "lucide-react";
import type { ShadowComparisonMetrics, TaskControlDecision } from "./types";

export default function ControlDecisionPanel({ decision, shadowComparison }: { decision?: TaskControlDecision; shadowComparison?: ShadowComparisonMetrics }) {
  if (!decision) return <section className="task-control-panel"><div className="task-control-panel-title"><BrainCircuit size={16} /><h3>控制决策</h3></div><p className="task-control-muted">当前没有待决策任务。</p></section>;
  return <section className="task-control-panel"><div className="task-control-panel-title"><BrainCircuit size={16} /><h3>控制决策</h3></div>
    <div className="control-decision-action"><span>{decision.action.kind}</span><ArrowRight size={15} /><strong>{decision.reason}</strong></div>
    <dl className="control-decision-details"><div><dt>证据</dt><dd>已满足 {decision.acceptance.satisfied} · 未满足 {decision.acceptance.unsatisfied} · 证据不足 {decision.acceptance.unknown}</dd></div><div><dt>风险/成本</dt><dd>{decision.expected_risk} / {decision.expected_cost}</dd></div><div><dt>来源</dt><dd>{decision.shadow ? "影子决策" : "串行控制器"}{decision.cache_hit ? " · 命中事实缓存" : ""}</dd></div></dl>
    {decision.shadow && shadowComparison && <div className="shadow-comparison-summary">
      <strong>影子对照</strong>
      <span>已评估 {shadowComparison.evaluated} · 一致 {shadowComparison.comparable_matches} · 差异 {shadowComparison.comparable_differences} · 不可比较 {shadowComparison.uncomparable}</span>
      {shadowComparison.latest && <small>{shadowComparison.latest.shadow_action} / {shadowComparison.latest.legacy_command || "等待或人工边界"} · {shadowComparison.latest.reason}</small>}
    </div>}
    <small className="task-control-fingerprint">决策 {decision.decision_id}</small>
  </section>;
}
