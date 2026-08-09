// src/MilestoneReviewPanel.tsx — 大阶段审阅 A/B/C 分支
import { useRef, useState } from "react";
import { CheckCircle2, GitBranch, RotateCcw } from "lucide-react";
import { ActionButton } from "./components/ActionButton";
import { MilestoneHumanReviewDialog } from "./MilestoneHumanReviewDialog";
import type { Milestone, MilestoneReviewSubmission } from "./types";

interface MilestoneReviewPanelProps {
  milestone: Milestone;
  projectRevision: number;
  onSubmit: (submission: MilestoneReviewSubmission) => Promise<void>;
  busy?: boolean;
}

export function MilestoneReviewPanel({
  milestone,
  projectRevision,
  onSubmit,
  busy = false,
}: MilestoneReviewPanelProps) {
  const [selected, setSelected] = useState<string | null>(null);
  const [dialogBranch, setDialogBranch] = useState<"A" | "B" | "C" | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const submittingRef = useRef(false);
  const interactionBusy = busy || submitting;
  const activeItems = milestone.human_review_items.filter(
    (item) => item.review_cycle === milestone.human_review_cycle,
  );

  const submitOnce = async (submission: MilestoneReviewSubmission) => {
    if (busy || submittingRef.current) return;
    submittingRef.current = true;
    setSubmitting(true);
    setSubmitError(null);
    try {
      await onSubmit(submission);
      setDialogBranch(null);
    } catch (error) {
      setSubmitError(error instanceof Error ? error.message : String(error));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  };

  const handleConfirm = async () => {
    if (interactionBusy) return;
    const branch = selected === "continue" ? "A" : selected === "fix" ? "B" : "C";
    if (activeItems.length > 0 || branch === "B") {
      setSubmitError(null);
      setDialogBranch(branch);
      return;
    }
    await submitOnce({
      milestone_id: milestone.id,
      review_cycle: milestone.human_review_cycle,
      expected_revision: projectRevision,
      review_fingerprint: milestone.human_review_fingerprint,
      branch,
      branch_reason: "",
      decisions: [],
    });
  };

  return (
    <div className="milestone-review-panel">
      <h2>大阶段「{milestone.title}」已完成</h2>
      <p>请选择下一步方向：</p>

      <div className="branch-cards">
        <button
          type="button"
          className={`branch-card ${selected === 'continue' ? 'selected' : ''}`}
          aria-pressed={selected === 'continue'}
          disabled={interactionBusy}
          onClick={() => { if (!interactionBusy) setSelected('continue'); }}
        >
          <div className="branch-card-icon"><CheckCircle2 size={24} /></div>
          <div>
            <div className="branch-card-title">A：正常继续</div>
            <div className="branch-card-desc">
              批准当前大阶段成果，继续推进下一个大阶段
            </div>
          </div>
        </button>

        <button
          type="button"
          className={`branch-card ${selected === 'fix' ? 'selected' : ''}`}
          aria-pressed={selected === 'fix'}
          disabled={interactionBusy}
          onClick={() => { if (!interactionBusy) setSelected('fix'); }}
        >
          <div className="branch-card-icon"><RotateCcw size={24} /></div>
          <div>
            <div className="branch-card-title">B：修正过去</div>
            <div className="branch-card-desc">
              与产品经理讨论问题，生成回退建议，预览影响后再执行回退
            </div>
          </div>
        </button>

        <button
          type="button"
          className={`branch-card ${selected === 'adjust' ? 'selected' : ''}`}
          aria-pressed={selected === 'adjust'}
          disabled={interactionBusy}
          onClick={() => { if (!interactionBusy) setSelected('adjust'); }}
        >
          <div className="branch-card-icon"><GitBranch size={24} /></div>
          <div>
            <div className="branch-card-title">C：调整未来</div>
            <div className="branch-card-desc">
              保留已完成大阶段，只重新生成后续大阶段（新阶段需经质检）
            </div>
          </div>
        </button>
      </div>

      {selected && (
        <ActionButton onClick={handleConfirm} loading={interactionBusy} loadingLabel="提交中" style={{ marginTop: '20px', maxWidth: '300px' }}>
          {selected === 'continue' ? '确认继续' :
           selected === 'fix' ? '开始讨论修正' : '重新生成后续'}
        </ActionButton>
      )}

      {submitError && <p className="settings-error" role="alert">{submitError}</p>}

      {dialogBranch && (
        <MilestoneHumanReviewDialog
          branch={dialogBranch}
          busy={interactionBusy}
          items={activeItems}
          onCancel={() => {
            setDialogBranch(null);
            setSubmitError(null);
          }}
          onSubmit={async (decisions, branchReason) => {
            await submitOnce({
              milestone_id: milestone.id,
              review_cycle: milestone.human_review_cycle,
              expected_revision: projectRevision,
              review_fingerprint: milestone.human_review_fingerprint,
              branch: dialogBranch,
              branch_reason: branchReason,
              decisions,
            });
          }}
        />
      )}
    </div>
  );
}
