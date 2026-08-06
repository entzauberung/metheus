import { FileCode2, GitBranch, Gauge, ShieldCheck } from "lucide-react";
import type { TaskContract } from "./types";

export default function TaskContractPanel({ contract }: { contract?: TaskContract }) {
  if (!contract) return <section className="task-control-panel"><h3>任务合同</h3><p className="task-control-muted">当前没有可选任务。</p></section>;
  return (
    <section className="task-control-panel">
      <div className="task-control-panel-title"><FileCode2 size={16} /><h3>{contract.title || "任务合同"}</h3></div>
      <p className="task-control-goal">{contract.goal || "未填写任务目标"}</p>
      <div className="task-control-meta-grid">
        <span><Gauge size={14} />复杂度 {contract.complexity}</span>
        <span><ShieldCheck size={14} />风险 {contract.risk}</span>
        <span><GitBranch size={14} />深度 {contract.depth}</span>
        <span>工作负载 {contract.workload_scale} · 最大拆分深度 {contract.max_split_depth}</span>
        <span>预算 {contract.budget.level} · 最多 {contract.budget.max_executor_turns} 执行轮</span>
        <span>重试 transport {contract.budget.max_transport_retries} · Doom Loop {contract.budget.max_doom_loop_retries}</span>
        <span>预计 {contract.budget.estimated_model_calls} 次模型调用</span>
      </div>
      <div className="task-control-section"><strong>允许路径</strong><code>{contract.allowed_file_paths.join("、") || "未声明"}</code></div>
      <div className="task-control-section"><strong>验收标准</strong><ul>{contract.acceptance_criteria.map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></div>
      <div className="task-control-section"><strong>停止规则</strong><ul>{contract.stop_rules.map((item, index) => <li key={`${index}-${item}`}>{item}</li>)}</ul></div>
      <small className="task-control-fingerprint">合同指纹 {contract.fingerprint}</small>
    </section>
  );
}
