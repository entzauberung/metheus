// src/PlanApprovalPanel.tsx — 方案审批页面（根据 draft_status 分发四种视图）
import { useState } from "react";
import { Project } from "./types";
import { ArrowLeft, FileText, ShieldCheck, CheckCircle, XCircle, Clock } from "lucide-react";
import { Modal } from "./components/Modal";
import { ActionButton } from "./components/ActionButton";
import { IconButton } from "./components/IconButton";

interface PlanApprovalPanelProps {
  project: Project;
  onReturnToDiscussion: () => void;
  onApprove: (draftId: string, generationRevision: number) => void;
  onReject: (draftId: string, feedback: string) => void;
  onEnterConsole: () => void;
  onReDiscuss?: () => void;
  isSubmitting: boolean;
}

export function PlanApprovalPanel({
  project,
  onReturnToDiscussion,
  onApprove,
  onReject,
  onEnterConsole,
  onReDiscuss,
  isSubmitting,
}: PlanApprovalPanelProps) {
  const draft = project.plan_draft;
  const [showApproveConfirm, setShowApproveConfirm] = useState(false);
  const [showRejectDialog, setShowRejectDialog] = useState(false);
  const [rejectFeedback, setRejectFeedback] = useState("");

  // 无草稿的异常状态
  if (!draft) {
    return (
      <div className="plan-approval-panel" style={{ padding: "24px", textAlign: "center" }}>
        <p style={{ color: "#cf222e" }}>
          当前没有方案草稿。请返回讨论并重新生成。
        </p>
        <ActionButton onClick={onReturnToDiscussion} variant="secondary">返回继续讨论</ActionButton>
      </div>
    );
  }

  const isPending = draft.draft_status === "Pending";
  const isApproved = draft.draft_status === "Approved";
  const busy = isSubmitting;

  // === 草稿待审批视图 ===
  if (isPending) {
    return (
      <div className="plan-approval-panel" style={{ padding: "16px 24px" }}>
        {/* 顶部返回 */}
        <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "16px" }}>
          <IconButton
            icon={<ArrowLeft size={16} />}
            tooltip="返回继续讨论"
            onClick={onReturnToDiscussion}
            disabled={busy}
          />
          <span style={{ fontSize: "13px", color: "#656d76" }}>返回继续讨论</span>
        </div>

        {/* 待审批标题 */}
        <div style={{
          background: "#fff8c5", border: "1px solid #d4a72c", borderRadius: "8px",
          padding: "16px", marginBottom: "16px",
        }}>
          <h2 style={{ margin: "0 0 4px 0", fontSize: "18px", color: "#664d03", display: "flex", alignItems: "center", gap: "8px" }}>
            <FileText size={20} />
            项目方案草稿待审批
          </h2>
          <p style={{ margin: 0, color: "#664d03", fontSize: "13px" }}>
            生成时间：{new Date(draft.generated_at).toLocaleString()}　|
            讨论修订号：{draft.generation_revision}
          </p>
        </div>

        {/* 方案内容 */}
        <div style={{
          border: "1px solid #d0d7de", borderRadius: "8px", padding: "16px",
          maxHeight: "400px", overflowY: "auto", marginBottom: "16px",
          background: "#f6f8fa",
        }}>
          <h3 style={{ display: "flex", alignItems: "center", gap: "6px", fontSize: "14px" }}>
            <FileText size={16} /> 项目方案
          </h3>
          <pre style={{ whiteSpace: "pre-wrap", fontFamily: "inherit", fontSize: "13px" }}>
            {draft.plan_content || "（方案内容为空）"}
          </pre>
          {draft.constitution_part1_draft && (
            <>
              <h3 style={{ display: "flex", alignItems: "center", gap: "6px", fontSize: "14px", marginTop: "16px" }}>
                <ShieldCheck size={16} /> 宪法第一部分草稿
              </h3>
              <pre style={{ whiteSpace: "pre-wrap", fontFamily: "inherit", fontSize: "13px", color: "#656d76" }}>
                {draft.constitution_part1_draft}
              </pre>
            </>
          )}
        </div>

        {/* 操作按钮 */}
        <div style={{ display: "flex", gap: "12px", justifyContent: "center" }}>
          <ActionButton
            onClick={() => setShowApproveConfirm(true)}
            disabled={busy}
            variant="primary"
          >
            {busy ? "批准中..." : "批准项目方案"}
          </ActionButton>
          <ActionButton
            onClick={() => { setRejectFeedback(""); setShowRejectDialog(true); }}
            disabled={busy}
            variant="danger"
          >
            驳回并继续讨论
          </ActionButton>
        </div>

        <p style={{ textAlign: "center", color: "#656d76", fontSize: "12px", marginTop: "12px" }}>
          返回讨论本身不会删除草稿；但只要发送新的需求消息，草稿和检查就会过期。
        </p>

        {/* 批准确认弹窗（使用 Modal 组件） */}
        <Modal
          isOpen={showApproveConfirm}
          onClose={() => { if (!busy) setShowApproveConfirm(false); }}
          title="确认批准项目方案"
          description="批准后将执行以下操作："
          lockClose={busy}
          isSubmitting={busy}
          actions={[
            { label: "取消", onClick: () => setShowApproveConfirm(false), variant: "secondary" },
            {
              label: busy ? "批准中..." : "确认批准",
              onClick: () => {
                setShowApproveConfirm(false);
                onApprove(draft.draft_id, draft.generation_revision);
              },
              variant: "primary",
            },
          ]}
        >
          <ul style={{ fontSize: "13px", color: "#656d76", paddingLeft: "20px", margin: 0 }}>
            <li>写入项目宪法第一部分（CONSTITUTION.md）</li>
            {project.entry_kind === "HalfProject" && (
              <li>保护已有宪法第一部分内容</li>
            )}
            <li>将项目方案标记为正式已批准</li>
            <li>批准后<strong>不会自动</strong>进入 Console</li>
            <li>你仍需手动点击"进入控制台"</li>
          </ul>
        </Modal>

        {/* 驳回反馈弹窗（使用 Modal 组件） */}
        <Modal
          isOpen={showRejectDialog}
          onClose={() => { if (!busy) setShowRejectDialog(false); }}
          title="驳回项目方案"
          description="请说明驳回原因，以便后续讨论更有针对性。"
          lockClose={busy}
          isSubmitting={busy}
          actions={[
            { label: "取消", onClick: () => setShowRejectDialog(false), variant: "secondary" },
            {
              label: busy ? "驳回中..." : "确认驳回",
              onClick: () => {
                if (rejectFeedback.trim()) {
                  setShowRejectDialog(false);
                  onReject(draft.draft_id, rejectFeedback.trim());
                }
              },
              variant: "danger",
              disabled: !rejectFeedback.trim(),
            },
          ]}
        >
          <textarea
            value={rejectFeedback}
            onChange={e => setRejectFeedback(e.target.value)}
            placeholder="请填写驳回原因（必填）..."
            disabled={busy}
            style={{
              width: "100%", minHeight: "80px", padding: "8px",
              border: "1px solid #d0d7de", borderRadius: "6px",
              fontSize: "13px", marginBottom: "8px", resize: "vertical",
              boxSizing: "border-box",
            }}
          />
        </Modal>
      </div>
    );
  }

  // === 已批准视图 ===
  if (isApproved) {
    const managedActive = project.workflow_state.managed_flow_state?.active === true;
    return (
      <div className="plan-approval-panel" style={{ padding: "24px" }}>
        {managedActive && (
          <div style={{ padding: "10px 14px", background: "#f0e6ff", border: "1px solid #6e40c9", borderRadius: "6px", fontSize: "13px", marginBottom: "12px", color: "#6e40c9" }}>
            🤖 <strong>托管层运行中</strong> — 方案已批准，托管层将自动进入控制台并推进大阶段审批。
          </div>
        )}
        <div className="plan-approved-banner" style={{
          background: "#dafbe1", border: "1px solid #1a7f37", borderRadius: "8px",
          padding: "16px", marginBottom: "16px",
        }}>
          <h2 style={{ margin: "0 0 8px 0", fontSize: "18px", display: "flex", alignItems: "center", gap: "8px" }}>
            <CheckCircle size={20} color="#1a7f37" />
            项目方案已批准
          </h2>
          <p style={{ margin: 0, color: "#1a7f37", fontSize: "14px" }}>
            宪法第 1 部分已写入项目目录。批准时间：
            {draft.approved_at ? new Date(draft.approved_at).toLocaleString() : "未知"}
          </p>
        </div>

        {/* 方案摘要 */}
        <div style={{
          border: "1px solid #d0d7de", borderRadius: "8px", padding: "16px",
          maxHeight: "300px", overflowY: "auto", marginBottom: "16px",
          background: "#f6f8fa",
        }}>
          <pre style={{ whiteSpace: "pre-wrap", fontFamily: "inherit", fontSize: "13px" }}>
            {draft.plan_content}
          </pre>
        </div>

        <div style={{ textAlign: "center" }}>
          <ActionButton
            onClick={onEnterConsole}
            disabled={busy}
            variant="primary"
          >
            {busy ? "进入中..." : "进入控制台"}
          </ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            点击后进入 Console 阶段，开始大阶段规划。
          </p>
          {onReDiscuss && (
            <>
              <div style={{ margin: "16px 0", borderTop: "1px solid #d0d7de" }} />
              <ActionButton
                onClick={onReDiscuss}
                disabled={busy}
                variant="danger"
              >
                重新讨论方案
              </ActionButton>
              <p style={{ color: "#656d76", fontSize: "11px", marginTop: "4px" }}>
                将已批准方案移入历史记录，清空检查结果，回到讨论模式。
              </p>
            </>
          )}
        </div>
      </div>
    );
  }

  // === 已过期 / 已驳回 / 已被替代视图 ===
  const isExpired = draft.draft_status === "Expired";
  const isRejected = draft.draft_status === "Rejected";
  const isSuperseded = draft.draft_status === "Superseded";

  const statusLabel = isExpired ? "已过期" : isRejected ? "已驳回" : "已被替代";
  const statusTime = isExpired ? draft.expired_at : isRejected ? draft.rejected_at : draft.superseded_at;
  const StatusIcon = isExpired || isSuperseded ? Clock : XCircle;

  return (
    <div className="plan-approval-panel" style={{ padding: "24px", textAlign: "center" }}>
      <div style={{
        background: "#fff1f0", border: "1px solid #cf222e", borderRadius: "8px",
        padding: "16px", marginBottom: "16px",
      }}>
        <h2 style={{ margin: "0 0 8px 0", fontSize: "18px", color: "#cf222e", display: "flex", alignItems: "center", justifyContent: "center", gap: "8px" }}>
          <StatusIcon size={20} />
          方案草稿{statusLabel}
        </h2>
        <p style={{ margin: 0, color: "#cf222e", fontSize: "13px" }}>
          草稿在 {statusTime ? new Date(statusTime).toLocaleString() : "未知时间"} {statusLabel}。
          请返回讨论，重新检查并生成新方案。
        </p>
      </div>
      <ActionButton
        onClick={onReturnToDiscussion}
        variant="secondary"
      >
        返回继续讨论
      </ActionButton>
    </div>
  );
}
