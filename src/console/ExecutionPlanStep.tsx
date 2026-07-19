import { ArrowRight, BadgeCheck, ClipboardList, Pause, RefreshCw, SearchCheck, WandSparkles } from "lucide-react";
import { Project, StagePlanCheckResult, Subtask } from "../types";
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
  onRegenerate: (source: "check_failed" | "approval_rejected") => void;
  autopilotActive?: boolean;
  autopilotRunning?: boolean;
  onAutopilotPause?: () => void;
}

function checkDetails(check: StagePlanCheckResult): string[] {
  return [...check.omissions, ...check.out_of_scope, ...check.not_executable, ...check.suggestions];
}

function PlanCards({ tasks }: { tasks: Subtask[] }) {
  return tasks.length > 0 ? <div className="candidate-list">{tasks.map((task) => (
    <StageCandidateCard key={task.id} title={`${task.order}. ${task.title}`} description={task.context_summary}
      fields={[
        { label: "目标", value: task.goal },
        { label: "文件范围", value: task.allowed_file_paths },
        { label: "新建文件", value: task.new_file_paths },
        { label: "验收标准", value: task.acceptance_criteria },
        { label: "停止规则", value: task.stop_rules },
      ]} />
  ))}</div> : <EmptyState title="执行计划为空" message="请生成或同步执行计划。" />;
}

export function ExecutionPlanStep(props: Props) {
  const step = props.project.workflow_state.current_step;
  const milestone = props.project.milestones.find((item) => item.id === props.project.current_milestone_id);
  const midStage = milestone?.mid_stages.find((item) => item.id === props.project.current_mid_stage_id);
  const tasks = midStage?.subtasks ?? [];
  const check = midStage?.plan_check_result;
  const autopilotRunning = props.autopilotRunning === true;

  const autopilotBanner = autopilotRunning ? (
    <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
      <FeedbackBanner type="info" message={`自动驾驶运行中：${props.project.workflow_state.autopilot_state?.last_action || "自动推进中..."}`} />
      {props.onAutopilotPause && (
        <ActionButton icon={<Pause size={16} />} variant="secondary" onClick={props.onAutopilotPause}>暂停自动驾驶</ActionButton>
      )}
    </div>
  ) : null;

  if (step === "PlanGeneration") return (
    <ConsoleStepShell icon={<ClipboardList />} title="生成执行计划" description={midStage?.title || "当前中阶段"}
      status={props.busy ? "progress" : "pending"} statusLabel={props.busy ? "生成中" : "待生成"}
      feedback={props.feedback} busy={props.busy}
      actions={autopilotRunning ? undefined : <WorkflowActionBar><ActionButton icon={<WandSparkles size={16} />} loading={props.busy} loadingLabel="生成中" onClick={props.onGenerate}>生成执行计划</ActionButton></WorkflowActionBar>}>
      {autopilotBanner}
      <p className="console-step-summary">任务数量由范围和验收标准决定。</p>
    </ConsoleStepShell>
  );

  if (step === "PlanCheck") return (
    <ConsoleStepShell icon={<SearchCheck />} title="执行计划检查" description={`${tasks.length} 个小阶段`}
      status={check ? (check.passed ? "success" : "failure") : "pending"} statusLabel={check ? (check.passed ? "检查通过" : "检查失败") : "待检查"}
      feedback={props.feedback} busy={props.busy}
      actions={autopilotRunning ? undefined : <WorkflowActionBar>
        <ActionButton icon={<SearchCheck size={16} />} loading={props.busy} disabled={tasks.length === 0} onClick={props.onCheck}>运行检查</ActionButton>
        <ActionButton icon={<RefreshCw size={16} />} variant="danger" loading={props.busy} onClick={() => props.onRegenerate("check_failed")}>重新生成</ActionButton>
      </WorkflowActionBar>}>
      {autopilotBanner}
      {check && <FeedbackBanner type={check.passed ? "success" : "error"} message={check.passed ? "检查通过" : "检查未通过"} details={checkDetails(check)} />}
      <PlanCards tasks={tasks} />
      {check && !check.passed && <textarea className="console-feedback-input" value={props.regenerationFeedback} onChange={(event) => props.setRegenerationFeedback(event.target.value)} placeholder="补充重新生成反馈" disabled={props.busy} />}
    </ConsoleStepShell>
  );

  const isApproved = midStage?.plan_approved_at != null && (midStage?.plan_revision ?? 0) > 0;
  const isAtApprovalStep = step === "PlanApproving";

  // If plan is approved but step hasn't transitioned to Execution (e.g., after refresh),
  // show a sync/continue button to let the user advance.
  const needsSyncToExecution = isApproved && isAtApprovalStep;

  return <ConsoleStepShell icon={<BadgeCheck />} title="批准执行计划" description={`${tasks.length} 个小阶段`}
    status={isApproved ? "success" : "pending"} statusLabel={isApproved ? "已批准" : "待批准"}
    feedback={props.feedback} busy={props.busy}
    actions={needsSyncToExecution ? (<WorkflowActionBar>
      <ActionButton icon={<ArrowRight size={16} />} onClick={props.onApprove}>进入执行</ActionButton>
    </WorkflowActionBar>) : (isApproved || autopilotRunning) ? undefined : (<WorkflowActionBar>
      <ActionButton icon={<BadgeCheck size={16} />} loading={props.busy} onClick={props.onApprove}>批准执行计划</ActionButton>
      <ActionButton icon={<RefreshCw size={16} />} variant="danger" onClick={() => props.setRegenerationModalOpen(true)}>驳回并重新生成</ActionButton>
    </WorkflowActionBar>)}>
    {autopilotBanner}
    {isApproved && <FeedbackBanner type="success" message="执行计划已冻结，已进入执行阶段。" />}
    {!isApproved && check?.passed && <FeedbackBanner type="success" message="检查已通过" />}
    <PlanCards tasks={tasks} />
    {!isApproved && (
      <Modal isOpen={props.regenerationModalOpen} onClose={() => props.setRegenerationModalOpen(false)} title="驳回并重新生成执行计划"
        description="已有执行事实时后端会拒绝覆盖。" isDanger lockClose={props.busy} isSubmitting={props.busy}
        actions={[
          { label: "取消", onClick: () => props.setRegenerationModalOpen(false), variant: "secondary", disabled: props.busy },
          { label: props.busy ? "重新生成中..." : "确认驳回", onClick: () => props.onRegenerate("approval_rejected"), variant: "danger", disabled: props.busy },
        ]}>
        <textarea className="console-feedback-input" value={props.regenerationFeedback} onChange={(event) => props.setRegenerationFeedback(event.target.value)} placeholder="驳回原因" disabled={props.busy} />
      </Modal>
    )}
  </ConsoleStepShell>;
}
