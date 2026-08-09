import { CheckCircle2, CircleAlert, HelpCircle, OctagonAlert } from "lucide-react";
import type { AcceptanceLedgerItem, Provability } from "./types";

const labels = {
  Satisfied: "已满足",
  AiProvisionallySatisfied: "AI 临时通过",
  DeferredHumanReview: "延期人工确认",
  Unsatisfied: "未满足",
  Unknown: "证据不足",
  Contradictory: "契约冲突",
  AcceptedDeviation: "接受偏差",
} as const;

function statusIcon(status: AcceptanceLedgerItem["status"]) {
  if (status === "Satisfied" || status === "AiProvisionallySatisfied") return <CheckCircle2 size={15} />;
  if (status === "Unsatisfied") return <OctagonAlert size={15} />;
  if (status === "Contradictory") return <CircleAlert size={15} />;
  return <HelpCircle size={15} />;
}

const provabilityLabels: Record<Provability, string> = {
  Deterministic: "确定性证明",
  AutomatedTest: "自动化测试",
  SemanticReview: "AI 语义审查",
  HumanReview: "人工确认",
  Unprovable: "不可自动证明",
};

export default function AcceptanceLedgerPanel({
  items,
  provabilityByIndex = {},
}: {
  items: AcceptanceLedgerItem[];
  provabilityByIndex?: Partial<Record<number, Provability>>;
}) {
  return <section className="task-control-panel"><div className="task-control-panel-title"><CheckCircle2 size={16} /><h3>验收账本</h3></div>
    {items.length === 0 ? <p className="task-control-muted">尚无逐项验收记录。</p> : <ul className="acceptance-ledger-list">{items.map(item => <li key={item.criterion_index} className={`ledger-${item.status.toLowerCase()}`}>
      <span>{statusIcon(item.status)}</span><div><strong>#{item.criterion_index} {labels[item.status]}</strong>
        {provabilityByIndex[item.criterion_index] && <small className="acceptance-provability-badge">{provabilityLabels[provabilityByIndex[item.criterion_index]!]}</small>}
        <p>{item.criterion}</p>{item.evidence && <small>{item.evidence}</small>}
        {item.evidence_references.length > 0 && <ul className="acceptance-evidence-references">{item.evidence_references.map(reference => <li key={`${reference.block_id}-${reference.file}-${reference.start_line ?? 0}`}>
          <code>{reference.block_id}</code>
          <span>{reference.file || "未标明文件"}{reference.start_line ? `:${reference.start_line}${reference.end_line && reference.end_line !== reference.start_line ? `-${reference.end_line}` : ""}` : ""}</span>
          <small>{reference.source_kind}</small>
        </li>)}</ul>}
      </div>
    </li>)}</ul>}
  </section>;
}
