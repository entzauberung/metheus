// src/PlanApprovalPanel.tsx — 方案审批页面（根据 draft_status 分发四种视图）
import { useState } from "react";
import { ManagedFlowState, Project } from "./types";
import { ArrowLeft, FileText, ShieldCheck, CheckCircle, XCircle, Clock } from "lucide-react";
import { Modal } from "./components/Modal";
import { ActionButton } from "./components/ActionButton";
import { IconButton } from "./components/IconButton";
import { getManagedFlowPresentation } from "./managedFlowPolicy";

interface PlanApprovalPanelProps {
  project: Project;
  onReturnToDiscussion: () => void;
  onApprove: (draftId: string, generationRevision: number) => void;
  onReject: (draftId: string, feedback: string) => void;
  onEnterConsole: () => void;
  onReDiscuss?: () => void;
  isSubmitting: boolean;
  /** 托管状态；有持久化事实时展示统一入口，ErrorStopped 不得消失 */
  managedFlowState?: ManagedFlowState | null;
  onStartManagedFlow?: () => void;
  onResumeManagedFlow?: () => void;
  onPauseManagedFlow?: () => void;
  onStopManagedFlow?: () => void;
}

export function PlanApprovalPanel({
  project,
  onReturnToDiscussion,
  onApprove,
  onReject,
  onEnterConsole,
  onReDiscuss,
  isSubmitting,
  managedFlowState,
  onStartManagedFlow,
  onResumeManagedFlow,
  onPauseManagedFlow,
  onStopManagedFlow,
}: PlanApprovalPanelProps) {
  const draft = project.plan_draft;
  const [showApproveConfirm, setShowApproveConfirm] = useState(false);
  const [showRejectDialog, setShowRejectDialog] = useState(false);
  const [rejectFeedback, setRejectFeedback] = useState("");

  const managedState = managedFlowState ?? project.workflow_state.managed_flow_state;
  const managedPresentation = managedState
    ? getManagedFlowPresentation(managedState, "PlanApproval")
    : null;
  const managedActive = managedState?.active === true;
  const managedCanStart = managedPresentation?.canStart ?? !managedState;

  const managedControls = managedState && managedPresentation ? (
    <div
      data-testid="plan-approval-managed-controls"
      style={{
        marginTop: "16px",
        marginBottom: "12px",
        padding: "12px 14px",
        borderRadius: "8px",
        border: `1px solid ${managedState.run_status === "ErrorStopped" ? "#cf222e" : "#6e40c9"}`,
        background: managedState.run_status === "ErrorStopped" ? "#fff1f0" : "#f0e6ff",
        color: managedState.run_status === "ErrorStopped" ? "#cf222e" : "#6e40c9",
        fontSize: "13px",
        textAlign: "left",
      }}
    >
      <p style={{ margin: "0 0 4px" }}>
        状态：{managedPresentation.statusLabel}
      </p>
      <p style={{ margin: "0 0 4px" }}>
        错误原因：{managedState.error_message || "暂无"}
      </p>
      <p style={{ margin: "0 0 8px" }}>
        下一步：{managedPresentation.nextStepLabel}
      </p>
      <div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
        {managedCanStart && onStartManagedFlow && (
          <ActionButton
            onClick={onStartManagedFlow}
            disabled={isSubmitting}
            variant="secondary"
          >
            启动托管
          </ActionButton>
        )}
        {managedPresentation.canResume && onResumeManagedFlow && (
          <ActionButton
            onClick={onResumeManagedFlow}
            disabled={isSubmitting}
            variant="secondary"
          >
            {managedPresentation.resumeLabel}
          </ActionButton>
        )}
        {managedPresentation.canPause && onPauseManagedFlow && (
          <ActionButton
            onClick={onPauseManagedFlow}
            disabled={isSubmitting}
            variant="ghost"
          >
            暂停托管
          </ActionButton>
        )}
        {managedActive && onStopManagedFlow && (
          <ActionButton
            onClick={onStopManagedFlow}
            disabled={isSubmitting}
            variant="ghost"
          >
            停止托管并转人工
          </ActionButton>
        )}
      </div>
    </div>
  ) : null;

  // 无草稿的异常状态
  if (!draft) {
    return (
      <div className="plan-approval-panel" style={{ padding: "24px", textAlign: "center" }}>
        <p style={{ color: "#cf222e" }}>
          当前没有方案草稿。请返回讨论并重新生成。
        </p>
        {managedControls}
        <ActionButton onClick={onReturnToDiscussion} variant="secondary">返回继续讨论</ActionButton>
      </div>
    );
  }

  const isPending = draft.draft_status === "Pending";
  const isApproved = draft.draft_status === "Approved";
  const busy = isSubmitting;
  const workloadProfile = project.workload_profile;
  const workloadBindingValid = Boolean(
    workloadProfile
    && workloadProfile.discussion_revision === project.discussion_revision
    && workloadProfile.fingerprint === draft.workload_profile_fingerprint,
  );
  const workloadProfileCard = workloadProfile ? (
    <div
      data-workload-profile={workloadProfile.scale}
      style={{
        border: `1px solid ${workloadBindingValid ? "#0969da" : "#cf222e"}`,
        borderRadius: "8px",
        padding: "14px",
        marginBottom: "16px",
        background: workloadBindingValid ? "#ddf4ff" : "#fff1f0",
        fontSize: "13px",
      }}
    >
      <strong>工作负载画像：{workloadProfile.scale}</strong>
      <div style={{ marginTop: "6px" }}>
        层级策略：{workloadProfile.use_mid_stage_layer
          ? "Milestone → MidStage → Subtask"
          : "Milestone → Subtask"}
        ；检查深度：{workloadProfile.check_depth}
      </div>
      <div style={{ marginTop: "4px" }}>
        数量上限：Milestone {workloadProfile.max_milestones}，
        MidStage {workloadProfile.max_mid_stages}，Subtask {workloadProfile.max_subtasks}
      </div>
      <ul style={{ margin: "8px 0 0", paddingLeft: "20px" }}>
        {workloadProfile.evidence.map((item, index) => (
          <li key={`${index}-${item}`}>{item}</li>
        ))}
      </ul>
      {!workloadBindingValid && (
        <p style={{ color: "#cf222e", margin: "8px 0 0" }}>
          当前画像已过期或与草稿指纹不一致，请重新完成目标完整性检查并生成方案。
        </p>
      )}
    </div>
  ) : (
    <div
      data-workload-profile="missing"
      style={{ border: "1px solid #cf222e", borderRadius: "8px", padding: "14px", marginBottom: "16px", background: "#fff1f0", color: "#cf222e" }}
    >
      工作负载画像缺失，请重新完成目标完整性检查；当前草稿不能批准。
    </div>
  );

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

        {workloadProfileCard}

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
            disabled={busy || !workloadBindingValid}
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

        {managedControls}

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
              disabled: !workloadBindingValid,
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
    return (
      <div className="plan-approval-panel" style={{ padding: "24px" }}>
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

        {workloadProfileCard}

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

        {managedControls}

        <div style={{ textAlign: "center" }}>
          <ActionButton
            onClick={onEnterConsole}
            disabled={busy || !workloadBindingValid}
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
      {managedControls}
      <ActionButton
        onClick={onReturnToDiscussion}
        variant="secondary"
      >
        返回继续讨论
      </ActionButton>
    </div>
  );
}
