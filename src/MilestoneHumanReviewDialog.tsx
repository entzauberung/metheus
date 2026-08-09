import { useMemo, useState } from "react";
import type {
  MilestoneHumanDecision,
  MilestoneHumanReviewDecisionSubmission,
  MilestoneHumanReviewItem,
} from "./types";
import { ActionButton } from "./components/ActionButton";

interface MilestoneHumanReviewDialogProps {
  branch: "A" | "B" | "C";
  items: MilestoneHumanReviewItem[];
  busy: boolean;
  onCancel: () => void;
  onSubmit: (
    decisions: MilestoneHumanReviewDecisionSubmission[],
    branchReason: string,
  ) => Promise<void>;
}

interface DraftDecision {
  decision: MilestoneHumanDecision;
  reason: string;
}

function statusLabel(status: MilestoneHumanReviewItem["ai_status"]): string {
  if (status === "AiProvisionallySatisfied") return "AI 临时认为满足";
  if (status === "DeferredHumanReview") return "AI 无法判断，延期人工确认";
  if (status === "Unsatisfied") return "AI 发现明确问题";
  return status;
}

export function MilestoneHumanReviewDialog({
  branch,
  items,
  busy,
  onCancel,
  onSubmit,
}: MilestoneHumanReviewDialogProps) {
  const [drafts, setDrafts] = useState<Record<string, DraftDecision>>({});
  const [branchReason, setBranchReason] = useState("");
  const groups = useMemo(() => {
    const grouped = new Map<string, MilestoneHumanReviewItem[]>();
    for (const item of items) {
      const group = grouped.get(item.task_id) ?? [];
      group.push(item);
      grouped.set(item.task_id, group);
    }
    return [...grouped.entries()];
  }, [items]);

  const allHandled = items.every((item) => {
    const decision = drafts[item.id]?.decision ?? item.human_decision;
    return decision === "Confirmed" || decision === "Rejected";
  });
  const hasRejected = items.some((item) => {
    const decision = drafts[item.id]?.decision ?? item.human_decision;
    return decision === "Rejected";
  });
  const acHasRejected = branch !== "B" && hasRejected;
  const bRequirementsMet = branch !== "B"
    || (hasRejected && branchReason.trim().length > 0);
  const canSubmit = allHandled && !acHasRejected && bRequirementsMet && !busy;

  const update = (item: MilestoneHumanReviewItem, decision: MilestoneHumanDecision) => {
    if (busy) return;
    setDrafts((current) => ({
      ...current,
      [item.id]: {
        decision,
        reason: current[item.id]?.reason ?? item.human_reason ?? "",
      },
    }));
  };

  const submit = async () => {
    if (!canSubmit) return;
    const decisions = items.map((item) => {
      const draft = drafts[item.id];
      const decision = draft?.decision ?? item.human_decision;
      if (decision !== "Confirmed" && decision !== "Rejected") {
        throw new Error(`人工确认项尚未处理：${item.id}`);
      }
      return {
        item_id: item.id,
        decision,
        reason: draft?.reason ?? item.human_reason ?? "",
      } satisfies MilestoneHumanReviewDecisionSubmission;
    });
    await onSubmit(decisions, branchReason.trim());
  };

  return (
    <div className="milestone-human-review-backdrop" role="presentation">
      <section
        aria-labelledby="milestone-human-review-title"
        aria-modal="true"
        className="milestone-human-review-dialog"
        role="dialog"
      >
        <header>
          <h3 id="milestone-human-review-title">集中人工确认 · 分支 {branch}</h3>
          <p>AI 结论仅供辅助。请逐项作出真实人工判断，未处理项不会提交。</p>
        </header>

        <div className="milestone-human-review-list">
          {groups.map(([taskId, taskItems]) => (
            <section className="milestone-human-review-group" key={taskId}>
              <h4>任务 {taskId}</h4>
              {taskItems.map((item) => {
                const draft = drafts[item.id];
                const decision = draft?.decision ?? item.human_decision;
                return (
                  <article className="milestone-human-review-item" key={item.id}>
                    <div className="milestone-human-review-copy">
                      <strong>{item.criterion_index}. {item.criterion}</strong>
                      <span>{statusLabel(item.ai_status)}</span>
                      <p>{item.ai_evidence || "没有可展示的 AI 证据"}</p>
                      <span>视觉辅助：{item.visual_status}</span>
                      <p>{item.visual_summary || "没有可用的视觉结论"}</p>
                      {item.visual_evidence.length > 0 && (
                        <small>
                          图片证据：{item.visual_evidence.map((evidence) => evidence.path).join("、")}
                        </small>
                      )}
                    </div>
                    <fieldset disabled={busy}>
                      <legend>人工结论</legend>
                      <label>
                        <input
                          checked={decision === "Confirmed"}
                          name={`decision-${item.id}`}
                          onChange={() => update(item, "Confirmed")}
                          type="radio"
                        />
                        确认
                      </label>
                      <label>
                        <input
                          checked={decision === "Rejected"}
                          name={`decision-${item.id}`}
                          onChange={() => update(item, "Rejected")}
                          type="radio"
                        />
                        标记问题
                      </label>
                    </fieldset>
                    <label className="milestone-human-review-reason">
                      说明（标记问题时建议填写）
                      <textarea
                        disabled={busy}
                        onChange={(event) => setDrafts((current) => ({
                          ...current,
                          [item.id]: {
                            decision,
                            reason: event.target.value,
                          },
                        }))}
                        value={draft?.reason ?? item.human_reason ?? ""}
                      />
                    </label>
                  </article>
                );
              })}
            </section>
          ))}
          {items.length === 0 && <p>当前大阶段没有延期人工确认项。</p>}
        </div>

        {branch === "B" && (
          <label className="milestone-human-review-reason">
            修正理由
            <textarea
              disabled={busy}
              onChange={(event) => setBranchReason(event.target.value)}
              placeholder="说明需要回退修正的原因（必填）"
              value={branchReason}
            />
          </label>
        )}

        {acHasRejected && <p className="settings-error">A/C 分支不能包含被拒绝项。</p>}
        {branch === "B" && !hasRejected && (
          <p className="settings-error">B 分支必须至少标记一个问题。</p>
        )}
        {branch === "B" && branchReason.trim().length === 0 && (
          <p className="settings-error">B 分支必须填写修正理由。</p>
        )}
        <footer className="milestone-human-review-actions">
          <button disabled={busy} onClick={onCancel} type="button">取消</button>
          <ActionButton
            disabled={!canSubmit}
            loading={busy}
            loadingLabel="原子提交中"
            onClick={submit}
          >
            提交人工结论与分支
          </ActionButton>
        </footer>
      </section>
    </div>
  );
}
