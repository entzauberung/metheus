import { ArrowRight, BadgeCheck, Boxes, RefreshCw, SearchCheck, WandSparkles } from "lucide-react";
import { Project } from "../types";
import { ActionButton } from "../components/ActionButton";
import { ConsoleFeedback, ConsoleStepShell } from "../components/ConsoleStepShell";
import { EmptyState } from "../components/EmptyState";
import { FeedbackBanner } from "../components/FeedbackBanner";
import { Modal } from "../components/Modal";
import { StageCandidateCard } from "../components/StageCandidateCard";
import { WorkflowActionBar } from "../components/WorkflowActionBar";

interface Props {
  project: Project; busy: boolean; feedback: ConsoleFeedback | null;
  regenerationFeedback: string; setRegenerationFeedback: (value: string) => void;
  regenerationModalOpen: boolean; setRegenerationModalOpen: (open: boolean) => void;
  onGenerate: () => void; onCheck: () => void; onApprove: () => void;
  onSelect: (id: string) => void; onContinue: () => void;
  onEnterNextStep: () => void;  // Phase 4: dedicated enter-execution/plan-generation action
  onRegenerate: (source: "check_failed" | "approval_rejected") => void;
  autopilotActive?: boolean;
  autopilotRunning?: boolean;
  onAutopilotPause?: () => void;
}

export function MidStagePlanningStep(props: Props) {
  const step = props.project.workflow_state.current_step;
  const milestone = props.project.milestones.find((item) => item.id === props.project.current_milestone_id);
  const autopilotRunning = props.autopilotRunning === true;
  const autopilotActive = props.autopilotActive === true;

  const autopilotBanner = (autopilotActive && !autopilotRunning) ? (
    <FeedbackBanner type="info" message="自动驾驶暂停中。" />
  ) : null;
  const draft = props.project.mid_stage_draft;
  const candidates = draft?.candidate_mid_stages ?? [];
  const cards = (items: typeof candidates, selectable = false) => (
    <div className="candidate-list">{items.map((item) => (
      <StageCandidateCard key={item.id} title={item.title} version={item.version} description={item.description}
        fields={[{ label: "技术重点", value: item.tech_focus }]}
        selected={props.project.current_mid_stage_id === item.id} readOnly={!selectable}
        onSelect={() => props.onSelect(item.id)} />
    ))}</div>
  );

  if (step === "MidStageGeneration") return (
    <ConsoleStepShell icon={<Boxes />} title="中阶段规划" description={milestone?.title || "当前大阶段"}
      status={props.busy ? "progress" : "pending"} statusLabel={props.busy ? "生成中" : "待生成"}
      feedback={props.feedback} busy={props.busy}
      actions={autopilotActive ? undefined : <WorkflowActionBar><ActionButton icon={<WandSparkles size={16} />} loading={props.busy} loadingLabel="生成中" onClick={props.onGenerate}>生成中阶段草稿</ActionButton></WorkflowActionBar>}>
      {autopilotBanner}
      <p className="console-step-summary">当前大阶段已选定，可以编译垂直切片。</p>
    </ConsoleStepShell>
  );

  if (step === "MidStageCheck") {
    const failed = draft?.status === "CheckFailed";
    return <ConsoleStepShell icon={<SearchCheck />} title="中阶段质量检查" description={`${candidates.length} 个候选中阶段`}
      status={failed ? "failure" : "pending"} statusLabel={failed ? "检查失败" : "待检查"}
      feedback={props.feedback} busy={props.busy}
      actions={autopilotActive ? undefined : <WorkflowActionBar>
        <ActionButton icon={<SearchCheck size={16} />} loading={props.busy} disabled={candidates.length === 0} onClick={props.onCheck}>运行检查</ActionButton>
        <ActionButton icon={<RefreshCw size={16} />} variant="danger" loading={props.busy} onClick={() => props.onRegenerate("check_failed")}>重新生成</ActionButton>
      </WorkflowActionBar>}>
      {autopilotBanner}
      {draft?.check_result && <FeedbackBanner type={failed ? "error" : "success"} message={failed ? "检查未通过" : "检查通过"} details={[draft.check_result]} />}
      {candidates.length > 0 ? cards(candidates) : <EmptyState title="没有中阶段草稿" message="请同步项目状态后重试。" />}
      {failed && <textarea className="console-feedback-input" value={props.regenerationFeedback} onChange={(event) => props.setRegenerationFeedback(event.target.value)} placeholder="补充重新生成反馈" disabled={props.busy} />}
    </ConsoleStepShell>;
  }

  if (step === "MidStageApproval") return (
    <ConsoleStepShell icon={<BadgeCheck />} title="批准中阶段" description="质量检查已通过" status="success" statusLabel="待批准"
      feedback={props.feedback} busy={props.busy}
      actions={autopilotActive ? undefined : <WorkflowActionBar>
        <ActionButton icon={<BadgeCheck size={16} />} loading={props.busy} onClick={props.onApprove}>批准中阶段</ActionButton>
        <ActionButton icon={<RefreshCw size={16} />} variant="danger" onClick={() => props.setRegenerationModalOpen(true)}>驳回并重新生成</ActionButton>
      </WorkflowActionBar>}>
      {autopilotBanner}
      {draft?.check_result && <FeedbackBanner type="success" message="检查通过" details={[draft.check_result]} />}
      {cards(candidates)}
      <Modal isOpen={props.regenerationModalOpen} onClose={() => props.setRegenerationModalOpen(false)} title="驳回并重新生成中阶段草稿"
        description="新草稿保存成功前会保留当前草稿。" isDanger lockClose={props.busy} isSubmitting={props.busy}
        actions={[
          { label: "取消", onClick: () => props.setRegenerationModalOpen(false), variant: "secondary", disabled: props.busy },
          { label: props.busy ? "重新生成中..." : "确认驳回", onClick: () => props.onRegenerate("approval_rejected"), variant: "danger", disabled: props.busy },
        ]}>
        <textarea className="console-feedback-input" value={props.regenerationFeedback} onChange={(event) => props.setRegenerationFeedback(event.target.value)} placeholder="驳回原因" disabled={props.busy} />
      </Modal>
    </ConsoleStepShell>
  );

  const formal = milestone?.mid_stages ?? [];
  const selectedMid = formal.find(m => m.id === props.project.current_mid_stage_id);
  const selectedIsCompleted = selectedMid?.status === "Completed";
  const hasCompletedMidStages = formal.some(m => m.status === "Completed");
  const pendingMidStages = formal.filter(m => m.status !== "Completed");

  // Phase 4: Build contextual action based on selected mid-stage state.
  // Selection (onSelect) is separate from entering the next step (onEnterNextStep / onContinue).
  let stepDescription = "选择正式中阶段后生成执行计划";
  let nextLabel: string | null = null;
  let nextDisabled = false;
  if (selectedMid) {
    if (selectedIsCompleted) {
      stepDescription = "该中阶段已完成。请选择其他未完成的中阶段。";
      nextDisabled = true;
    } else if (selectedMid.plan_approved_at && (selectedMid.plan_revision ?? 0) > 0) {
      nextLabel = "进入执行阶段";
      stepDescription = "该中阶段执行计划已批准，可以进入执行。";
    } else if (selectedMid.subtasks?.some(s => s.status === "Executing" || s.status === "AwaitingConfirmation" || s.status === "Passed")) {
      nextLabel = "回到执行阶段";
      stepDescription = "该中阶段已有执行记录，将回到执行步骤。";
    } else {
      nextLabel = "生成执行计划";
      stepDescription = "已选择中阶段，可以生成执行计划。";
    }
  }

  return <ConsoleStepShell icon={<Boxes />} title="选择中阶段" description={stepDescription}
    status={props.project.current_mid_stage_id ? (selectedIsCompleted ? "success" : "success") : "pending"}
    statusLabel={props.project.current_mid_stage_id ? (selectedIsCompleted ? "已完成" : "已选择") : "待选择"}
    feedback={props.feedback} busy={props.busy}
    actions={(props.project.current_mid_stage_id && !autopilotActive) ? <WorkflowActionBar>
      {!selectedIsCompleted && nextLabel && (
        <ActionButton icon={<ArrowRight size={16} />} disabled={nextDisabled}
          onClick={props.onEnterNextStep}>{nextLabel}</ActionButton>
      )}
    </WorkflowActionBar> : undefined}>
    {autopilotBanner}
    {formal.length > 0 ? (
      <div className="candidate-list">
        {formal.map((item) => {
          const isCompleted = item.status === "Completed";
          return (
            <StageCandidateCard key={item.id} title={item.title} version={item.version}
              description={isCompleted ? `✅ 已完成 — ${item.description}` : item.description}
              fields={[
                { label: "技术重点", value: item.tech_focus },
                ...(isCompleted ? [{ label: "状态", value: `已完成${item.completed_at ? ` (${new Date(item.completed_at).toLocaleDateString()})` : ""}` }] : []),
              ]}
              selected={props.project.current_mid_stage_id === item.id}
              readOnly={isCompleted}
              onSelect={isCompleted ? undefined : () => props.onSelect(item.id)} />
          );
        })}
      </div>
    ) : <EmptyState title="没有正式中阶段" message="请先完成中阶段批准。" />}
    {hasCompletedMidStages && pendingMidStages.length === 0 && (
      <FeedbackBanner type="success" message="当前大阶段所有中阶段已完成，可进入大阶段审阅。" />
    )}
  </ConsoleStepShell>;
}
