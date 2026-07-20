import { ArrowRight, BadgeCheck, Layers3, RefreshCw, SearchCheck, WandSparkles } from "lucide-react";
import { Project } from "../types";
import { ActionButton } from "../components/ActionButton";
import { ConsoleFeedback, ConsoleStepShell } from "../components/ConsoleStepShell";
import { EmptyState } from "../components/EmptyState";
import { FeedbackBanner } from "../components/FeedbackBanner";
import { Modal } from "../components/Modal";
import { StageCandidateCard } from "../components/StageCandidateCard";
import { WorkflowActionBar } from "../components/WorkflowActionBar";
import { DEEPSEEK_MODEL_DISPLAY_NAME } from "../constants";

interface Props {
  project: Project;
  busy: boolean;
  feedback: ConsoleFeedback | null;
  regenerationFeedback: string;
  setRegenerationFeedback: (value: string) => void;
  regenerationModalOpen: boolean;
  setRegenerationModalOpen: (open: boolean) => void;
  onGenerate: () => void;
  onCheck: () => void;
  onApprove: () => void;
  onApproveFuture: () => void;
  onSelect: (id: string) => void;
  onContinue: () => void;
  onRegenerate: (source: "check_failed" | "approval_rejected") => void;
  onSync: () => void;
}

export function MilestonePlanningStep(props: Props) {
  const { project, busy, feedback } = props;
  const step = project.workflow_state.current_step;
  const draft = project.milestone_draft;
  const candidates = draft?.candidate_milestones ?? [];
  const renderCandidates = (selectable = false) => (
    <div className="candidate-list">
      {(selectable ? project.milestones : candidates).map((milestone) => (
        <StageCandidateCard
          key={milestone.id}
          title={milestone.title}
          version={milestone.version}
          description={milestone.description}
          selected={project.current_milestone_id === milestone.id}
          readOnly={!selectable || busy}
          onSelect={busy ? undefined : () => props.onSelect(milestone.id)}
          fields={[
            { label: "目标", value: milestone.goal },
            { label: "范围", value: milestone.scope },
            { label: "依赖", value: milestone.dependencies },
            { label: "预期输出", value: milestone.expected_output },
            { label: "验收标准", value: milestone.acceptance_criteria },
          ]}
        />
      ))}
    </div>
  );

  if (step === "MilestoneGeneration") {
    return (
      <ConsoleStepShell icon={<Layers3 />} title="大阶段规划"
        description={`由 ${DEEPSEEK_MODEL_DISPLAY_NAME} 编译已批准方案`}
        status={busy ? "progress" : "pending"} statusLabel={busy ? "生成中" : "待生成"}
        feedback={feedback} busy={busy}
        actions={<WorkflowActionBar>
          <ActionButton icon={<WandSparkles size={16} />} loading={busy} loadingLabel="生成中" onClick={props.onGenerate}>生成大阶段草稿</ActionButton>
        </WorkflowActionBar>}>
        <p className="console-step-summary">项目方案已就绪，可以开始编译候选大阶段。</p>
      </ConsoleStepShell>
    );
  }

  if (step === "MilestoneCheck") {
    const failed = draft?.status === "CheckFailed";
    return (
      <ConsoleStepShell icon={<SearchCheck />} title="大阶段质量检查"
        description={`${candidates.length} 个候选大阶段`}
        status={failed ? "failure" : "pending"} statusLabel={failed ? "检查失败" : "待检查"}
        feedback={feedback} busy={busy}
        actions={<WorkflowActionBar>
          <ActionButton icon={<SearchCheck size={16} />} loading={busy} disabled={candidates.length === 0} onClick={props.onCheck}>运行检查</ActionButton>
          <ActionButton icon={<RefreshCw size={16} />} variant="danger" loading={busy} onClick={() => props.onRegenerate("check_failed")}>重新生成</ActionButton>
        </WorkflowActionBar>}>
        {draft?.check_result && <FeedbackBanner type={failed ? "error" : "success"} message={failed ? "检查未通过" : "检查通过"} details={[draft.check_result]} />}
        {candidates.length > 0 ? renderCandidates() : <EmptyState title="草稿数据缺失" message="工作流已进入检查步骤，但候选大阶段为空。" actionLabel="同步项目状态" onAction={props.onSync} />}
        {failed && <textarea className="console-feedback-input" value={props.regenerationFeedback} onChange={(event) => props.setRegenerationFeedback(event.target.value)} placeholder="补充重新生成反馈；留空时使用检查结果" disabled={busy} />}
      </ConsoleStepShell>
    );
  }

  if (step === "MilestoneApproval") {
    return (
      <ConsoleStepShell icon={<BadgeCheck />} title="批准大阶段"
        description="质量检查已通过" status="success" statusLabel="待批准"
        feedback={feedback} busy={busy}
        actions={<WorkflowActionBar>
          <ActionButton icon={<BadgeCheck size={16} />} loading={busy} onClick={props.onApprove}>批准大阶段</ActionButton>
          <ActionButton icon={<RefreshCw size={16} />} variant="danger" onClick={() => props.setRegenerationModalOpen(true)}>驳回并重新生成</ActionButton>
        </WorkflowActionBar>}>
        {draft?.check_result && <FeedbackBanner type="success" message="检查通过" details={[draft.check_result]} />}
        {renderCandidates()}
        <Modal isOpen={props.regenerationModalOpen} onClose={() => props.setRegenerationModalOpen(false)}
          title="驳回并重新生成大阶段草稿" description="新草稿保存成功前会保留当前草稿。"
          isDanger lockClose={busy} isSubmitting={busy}
          actions={[
            { label: "取消", onClick: () => props.setRegenerationModalOpen(false), variant: "secondary", disabled: busy },
            { label: busy ? "重新生成中..." : "确认驳回", onClick: () => props.onRegenerate("approval_rejected"), variant: "danger", disabled: busy },
          ]}>
          <textarea className="console-feedback-input" value={props.regenerationFeedback} onChange={(event) => props.setRegenerationFeedback(event.target.value)} placeholder="驳回原因" disabled={busy} />
        </Modal>
      </ConsoleStepShell>
    );
  }

  if (step === "FuturePlanApproval") {
    // Find retained milestones from project data
    const retainedIds = new Set(draft?.retained_milestone_ids ?? []);
    const retainedMilestones = project.milestones.filter(m => retainedIds.has(m.id));
    const isFutureOnly = draft?.draft_kind === "FutureOnly";
    const versionsNormalized = draft?.versions_normalized === true;

    // Count change info
    const originalRemaining = draft?.original_remaining_count;
    const newFutureCount = draft?.new_future_count;
    const countExpanded = draft?.count_expansion_warning === true;

    return (
      <ConsoleStepShell icon={<BadgeCheck />} title="批准后续大阶段" description="已完成大阶段保持不变，仅调整未来规划"
        status="pending" statusLabel="待批准" feedback={feedback} busy={busy}
        actions={<WorkflowActionBar><ActionButton icon={<BadgeCheck size={16} />} loading={busy} onClick={props.onApproveFuture}>批准后续大阶段</ActionButton></WorkflowActionBar>}>
        {!isFutureOnly && (
          <FeedbackBanner type="warning" message="当前草稿缺少 FutureOnly 元数据，请重新生成。" />
        )}

        {/* 数量变化摘要 */}
        {originalRemaining != null && newFutureCount != null && (
          <div style={{ fontSize: "13px", color: "#656d76", marginBottom: "12px", padding: "8px 12px", background: "#f6f8fa", borderRadius: "6px" }}>
            原后续大阶段：{originalRemaining} 个 → 新规划：{newFutureCount} 个
            {countExpanded && (
              <span style={{ color: "#9a6700", marginLeft: "8px" }}>⚠ 数量显著增加，请确认是否已明确要求扩展范围</span>
            )}
          </div>
        )}

        {/* 数量膨胀预警 */}
        {countExpanded && (
          <FeedbackBanner type="warning" message={`未来大阶段从 ${originalRemaining} 个扩展到 ${newFutureCount} 个。如果未明确要求扩展范围，建议返回讨论重新生成。`} />
        )}

        {/* 粒度问题 */}
        {draft?.granularity_issues && draft.granularity_issues.length > 0 && (
          <FeedbackBanner type="error" message="粒度校验发现问题" details={draft.granularity_issues} />
        )}

        {/* === 保留阶段（只读） === */}
        {retainedMilestones.length > 0 && (
          <div className="future-plan-section">
            <div className="future-plan-section-header">
              <span className="future-plan-section-badge retained">已保留</span>
              <span className="future-plan-section-desc">以下大阶段已完成，保持不变</span>
            </div>
            <div className="candidate-list retained-list">
              {retainedMilestones.map((milestone) => (
                <StageCandidateCard
                  key={milestone.id}
                  title={milestone.title}
                  version={milestone.version}
                  description={milestone.description}
                  selected={false}
                  readOnly={true}
                  fields={[
                    { label: "目标", value: milestone.goal },
                    { label: "状态", value: [milestone.status] },
                  ]}
                />
              ))}
            </div>
          </div>
        )}

        {/* === 分割线 === */}
        {retainedMilestones.length > 0 && candidates.length > 0 && (
          <div className="future-plan-divider">
            <div className="future-plan-divider-line" />
            <span className="future-plan-divider-label">以上为保留阶段 · 以下为新规划未来阶段</span>
            <div className="future-plan-divider-line" />
          </div>
        )}

        {/* === 未来候选阶段 === */}
        {candidates.length > 0 && (
          <div className="future-plan-section">
            <div className="future-plan-section-header">
              <span className="future-plan-section-badge future">新规划</span>
              <span className="future-plan-section-desc">
                系统归一化版本
                {draft?.normalized_versions && draft.normalized_versions.length > 0 && (
                  <code className="future-plan-versions">{draft.normalized_versions.join(" → ")}</code>
                )}
              </span>
            </div>
            {versionsNormalized && draft?.original_ai_versions && draft.original_ai_versions.length > 0 && (
              <div className="future-plan-ai-versions-note">
                AI 原始版本参考：{draft.original_ai_versions.join(", ")}（已归一化）
              </div>
            )}
            {renderCandidates()}
          </div>
        )}

        {candidates.length === 0 && <EmptyState title="后续草稿为空" message="请同步项目状态后重试。" actionLabel="同步项目状态" onAction={props.onSync} />}
      </ConsoleStepShell>
    );
  }

  return (
    <ConsoleStepShell icon={<Layers3 />} title="选择大阶段" description="选择正式大阶段后继续规划"
      status={project.current_milestone_id ? "success" : "pending"} statusLabel={project.current_milestone_id ? "已选择" : "待选择"}
      feedback={feedback} busy={busy}
      actions={project.current_milestone_id && !busy ? <WorkflowActionBar>
        <ActionButton icon={<ArrowRight size={16} />} onClick={props.onContinue}>开始中阶段规划</ActionButton>
      </WorkflowActionBar> : undefined}>
      {project.milestones.length > 0 ? renderCandidates(true) : <EmptyState title="没有正式大阶段" message="请先完成大阶段批准。" />}
    </ConsoleStepShell>
  );
}
