import { CheckCircle2, CircleAlert, HelpCircle, OctagonAlert } from "lucide-react";
import type { RuntimeOutcomePresentation } from "./runtimeOutcomePresentation";
import type { AcceptanceLedgerItem, Provability } from "./types";
import type { Subtask } from "./types";

const labels = {
  Satisfied: "已满足",
  AiProvisionallySatisfied: "AI 临时通过",
  DeferredHumanReview: "延期人工确认",
  Unsatisfied: "未满足",
  Unknown: "证据不足",
  Contradictory: "契约冲突",
  AcceptedDeviation: "接受偏差",
} as const;

const statusReasons: Partial<Record<AcceptanceLedgerItem["status"], string>> = {
  Unknown: "后端尚未提供足够证据",
  Unsatisfied: "后端证据未满足该验收标准",
  Contradictory: "验收契约与当前证据冲突",
  DeferredHumanReview: "等待人工确认该验收标准",
  AiProvisionallySatisfied: "仅为临时通过，尚未形成最终验收事实",
};

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

export type LedgerEmptyState =
  | "NotRequired"
  | "WaitingInitialization"
  | "AwaitingVerification"
  | "StateAnomaly"
  | null;

export function resolveLedgerEmptyState(
  items: AcceptanceLedgerItem[],
  criteria: string[],
  taskStatus?: Subtask["status"],
): LedgerEmptyState {
  if (criteria.length === 0) return items.length === 0 ? "NotRequired" : "StateAnomaly";
  const seenIndexes = new Set<number>();
  const malformed = items.some(item => {
    const index = item.criterion_index - 1;
    if (seenIndexes.has(item.criterion_index)) return true;
    seenIndexes.add(item.criterion_index);
    return index < 0 || index >= criteria.length || criteria[index] !== item.criterion;
  });
  if (malformed) return "StateAnomaly";
  const coveredCriteria = new Set(items.map(item => item.criterion_index));
  const missingCount = criteria.filter((_, index) => !coveredCriteria.has(index + 1)).length;
  if (items.length > 0 && missingCount === 0) return null;
  if (items.length > 0) return "StateAnomaly";
  if (taskStatus === "Pending") return "WaitingInitialization";
  if (taskStatus === "Executing" || taskStatus === "AwaitingConfirmation") {
    return "AwaitingVerification";
  }
  return "StateAnomaly";
}

export default function AcceptanceLedgerPanel({
  items,
  criteria = [],
  taskStatus,
  runtimeOutcome,
  provabilityByIndex = {},
}: {
  items: AcceptanceLedgerItem[];
  criteria?: string[];
  taskStatus?: Subtask["status"];
  runtimeOutcome?: RuntimeOutcomePresentation;
  provabilityByIndex?: Partial<Record<number, Provability>>;
}) {
  const emptyState = resolveLedgerEmptyState(items, criteria, taskStatus);
  const syncHint = runtimeOutcome && !runtimeOutcome.writeAllowed
    ? ` ${runtimeOutcome.writeBlockedReason}。`
    : "";
  let emptyLedgerMessage = "";
  if (emptyState) {
    if (emptyState === "NotRequired") {
      emptyLedgerMessage = "当前任务没有验收标准，无需建立逐项验收账本。";
    } else if (emptyState === "WaitingInitialization") {
      emptyLedgerMessage = `验收标准已存在，账本等待后端在验证阶段初始化。${syncHint}`;
    } else if (emptyState === "AwaitingVerification") {
      emptyLedgerMessage = `任务等待验证，后端尚未形成逐项验收记录。${syncHint}`;
    } else {
      emptyLedgerMessage = items.length === 0
        ? `验收标准已存在，但当前状态不应出现空账本；请同步状态并等待后端处理异常。${syncHint}`
        : `验收账本未完整覆盖标准，或存在重复/错配记录；已保留原始条目，请同步状态并等待后端补齐。${syncHint}`;
    }
  }
  return <section className="task-control-panel" data-ledger-empty-state={emptyState ?? undefined}><div className="task-control-panel-title"><CheckCircle2 size={16} /><h3>验收账本</h3></div>
    {emptyLedgerMessage && <p className="task-control-muted">{emptyLedgerMessage}</p>}
    {items.length > 0 && <ul className="acceptance-ledger-list">{items.map((item, index) => <li key={`${item.criterion_index}-${item.updated_at}-${index}`} className={`ledger-${item.status.toLowerCase()}`}>
      <span>{statusIcon(item.status)}</span><div><strong>#{item.criterion_index} {labels[item.status]}</strong>
        {provabilityByIndex[item.criterion_index] && <small className="acceptance-provability-badge">{provabilityLabels[provabilityByIndex[item.criterion_index]!]}</small>}
        <p>{item.criterion}</p>{(item.evidence || statusReasons[item.status]) && <small>{item.evidence || statusReasons[item.status]}</small>}
        {item.evidence_references.length > 0 && <ul className="acceptance-evidence-references">{item.evidence_references.map(reference => <li key={`${reference.block_id}-${reference.file}-${reference.start_line ?? 0}`}>
          <code>{reference.block_id}</code>
          <span>{reference.file || "未标明文件"}{reference.start_line ? `:${reference.start_line}${reference.end_line && reference.end_line !== reference.start_line ? `-${reference.end_line}` : ""}` : ""}</span>
          <small>{reference.source_kind}</small>
        </li>)}</ul>}
      </div>
    </li>)}</ul>}
  </section>;
}
