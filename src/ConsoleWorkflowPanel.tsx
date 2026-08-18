import { useState } from "react";
import { Activity, Clock3, Pause, Play, Square, Target } from "lucide-react";
import { ExecutionWorkspaceStatus, Project, RuntimeMutationResult } from "./types";
import { invokeWithTimeout, isInvokeTimeoutError } from "./utils/invokeWithTimeout";
import { ConsoleFeedback } from "./components/ConsoleStepShell";
import { MilestonePlanningStep } from "./console/MilestonePlanningStep";
import { MidStagePlanningStep } from "./console/MidStagePlanningStep";
import { ExecutionPlanStep } from "./console/ExecutionPlanStep";
import { getManagedFlowPresentation } from "./managedFlowPolicy";
import { resolvePlanTarget } from "./planTargetPolicy";
import { NativeReadinessPanel } from "./PreflightPanel";

interface Props {
  project: Project;
  onRuntimeMutation: (result: RuntimeMutationResult) => void;
  externalBusy: boolean;
  onActionStart: (action: string) => boolean;
  onActionEnd: () => void;
  onFeedback: (feedback: ConsoleFeedback | null) => void;
  workspaceStatus: ExecutionWorkspaceStatus | null;
  onPrepareWorkspace: () => Promise<void>;
  onPauseManagedFlow?: () => Promise<void>;
  onResumeManagedFlow?: () => Promise<void>;
  onStopManagedFlow?: () => Promise<void>;
}

type RegenerationSource = "check_failed" | "approval_rejected";

export function ConsoleWorkflowPanel({
  project, onRuntimeMutation, externalBusy, onActionStart, onActionEnd, onFeedback,
  workspaceStatus, onPrepareWorkspace,
  onPauseManagedFlow, onResumeManagedFlow, onStopManagedFlow,
}: Props) {
  const step = project.workflow_state.current_step;
  const busy = externalBusy;
  const [feedback, setLocalFeedback] = useState<ConsoleFeedback | null>(null);
  const [regenerationFeedback, setRegenerationFeedback] = useState("");
  const [milestoneModalOpen, setMilestoneModalOpen] = useState(false);
  const [midStageModalOpen, setMidStageModalOpen] = useState(false);
  const [planModalOpen, setPlanModalOpen] = useState(false);

  const setFeedback = (next: ConsoleFeedback | null) => {
    setLocalFeedback(next);
    onFeedback(next);
  };

  const beginAction = (action: string) => !busy && onActionStart(action);

  const syncProject = async () => {
    const result = await invokeWithTimeout<RuntimeMutationResult>("reconcile_managed_milestone_state_runtime", {
      projectName: project.name,
    });
    onRuntimeMutation(result);
    return result.runtime_snapshot.project;
  };

  const coordinate = async (isComplete: (latest: Project) => boolean) => {
    for (let attempt = 0; attempt < 6; attempt += 1) {
      if (attempt > 0) await new Promise((resolve) => setTimeout(resolve, 5000));
      try {
        const latest = await syncProject();
        if (isComplete(latest)) return true;
      } catch (error) {
        console.warn("协调 Console 项目状态失败", error);
      }
    }
    return false;
  };

  const runProjectCommand = async (
    command: string,
    args: Record<string, unknown>,
    successMessage: string,
  ) => {
    if (!beginAction(command)) return;
    setFeedback(null);
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>(`${command}_runtime`, args);
      onRuntimeMutation(result);
      setFeedback({ type: "success", message: successMessage });
    } catch (error) {
      setFeedback({ type: "error", message: String(error) });
    } finally {
      onActionEnd();
    }
  };

  const handleGenerateMilestone = async () => {
    if (!beginAction("generate_milestone_draft")) return;
    const startingRevision = project.workflow_state.data_revision;
    setFeedback(null);
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>("generate_milestone_draft_runtime", { projectName: project.name });
      onRuntimeMutation(result);
      setFeedback({ type: "success", message: "大阶段草稿已生成，请运行质量检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        setFeedback({ type: "info", message: "请求等待超时，正在同步后端最终状态。" });
        const done = await coordinate((latest) => latest.workflow_state.data_revision > startingRevision && latest.workflow_state.current_step === "MilestoneCheck");
        if (done) setFeedback({ type: "success", message: "已同步后端完成的大阶段草稿。" });
        else {
          setFeedback({ type: "info", message: "后端未完成，请稍后手动同步项目状态。" });
        }
      } else setFeedback({ type: "error", message: "生成失败：" + String(error) });
    } finally { onActionEnd(); }
  };

  const handleRegenerateMilestone = async (source: RegenerationSource) => {
    const draft = project.milestone_draft;
    if (!draft || !beginAction("regenerate_milestone_draft")) return;
    const revision = project.workflow_state.data_revision;
    const draftId = draft.draft_id;
    setFeedback(null);
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>("regenerate_milestone_draft_runtime", {
        projectName: project.name, currentDraftId: draftId,
        expectedDataRevision: revision, feedback: regenerationFeedback, source,
      });
      onRuntimeMutation(result);
      setRegenerationFeedback(""); setMilestoneModalOpen(false);
      setFeedback({ type: "success", message: "大阶段草稿已重新生成，请重新检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        const done = await coordinate((latest) => latest.workflow_state.data_revision > revision && latest.milestone_draft?.draft_id !== draftId);
        if (done) { setMilestoneModalOpen(false); setFeedback({ type: "success", message: "已同步新大阶段草稿。" }); }
        else { setFeedback({ type: "info", message: "后端未完成，请稍后同步项目状态。" }); }
      } else setFeedback({ type: "error", message: "重新生成失败：" + String(error) });
    } finally { onActionEnd(); }
  };

  const handleRegenerateMidStage = async (source: RegenerationSource) => {
    const draft = project.mid_stage_draft;
    if (!draft || !beginAction("regenerate_mid_stage_draft")) return;
    const revision = project.workflow_state.data_revision;
    const draftId = draft.draft_id;
    setFeedback(null);
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>("regenerate_mid_stage_draft_runtime", {
        projectName: project.name, currentDraftId: draftId,
        expectedDataRevision: revision, feedback: regenerationFeedback, source,
      });
      onRuntimeMutation(result);
      setRegenerationFeedback(""); setMidStageModalOpen(false);
      setFeedback({ type: "success", message: "中阶段草稿已重新生成，请重新检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        const done = await coordinate((latest) => latest.workflow_state.data_revision > revision && latest.mid_stage_draft?.draft_id !== draftId);
        if (done) { setMidStageModalOpen(false); setFeedback({ type: "success", message: "已同步新中阶段草稿。" }); }
        else setFeedback({ type: "info", message: "后端未完成，请稍后同步项目状态。" });
      } else setFeedback({ type: "error", message: "重新生成失败：" + String(error) });
    } finally { onActionEnd(); }
  };

  const handleRegeneratePlan = async (source: RegenerationSource) => {
    const target = resolvePlanTarget(project);
    if (!target || !beginAction("regenerate_execution_plan")) return;
    const revision = project.workflow_state.data_revision;
    const planRevision = target.planDraftRevision;
    setFeedback(null);
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>("regenerate_execution_plan_runtime", {
        projectName: project.name, expectedDataRevision: revision,
        expectedPlanDraftRevision: planRevision, feedback: regenerationFeedback, source,
      });
      onRuntimeMutation(result);
      setRegenerationFeedback(""); setPlanModalOpen(false);
      setFeedback({ type: "success", message: "执行计划已重新生成，请重新检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        const done = await coordinate((latest) => {
          const latestTarget = resolvePlanTarget(latest);
          return latest.workflow_state.data_revision > revision
            && (latestTarget?.planDraftRevision ?? 0) > planRevision;
        });
        if (done) { setPlanModalOpen(false); setFeedback({ type: "success", message: "已同步新执行计划。" }); }
        else setFeedback({ type: "info", message: "后端未完成，请稍后同步项目状态。" });
      } else setFeedback({ type: "error", message: "重新生成失败：" + String(error) });
    } finally { onActionEnd(); }
  };

  const autopilotRunning = project.workflow_state.autopilot_active === true
    && project.workflow_state.autopilot_state?.run_status === "Running";
  const managedActive = project.workflow_state.managed_flow_state?.active === true;
  const managedState = project.workflow_state.managed_flow_state;
  const managedRunning = managedActive && managedState?.run_status === "Running";
  const planningBusy = busy || autopilotRunning || managedRunning;
  const managedPresentation = managedState
    ? getManagedFlowPresentation(managedState, step, project.milestone_draft)
    : null;

  // Managed flow banner (shown during any Console step when a persisted state exists)
  const managedBanner = managedState ? (
    <div className={`managed-flow-banner managed-${managedState?.run_status.toLowerCase()}`}>
      <div className="managed-flow-summary">
        <div className="managed-flow-facts">
          <strong><Activity size={15} />{managedPresentation?.statusLabel}</strong>
          <span><Target size={14} />目标：{managedPresentation?.targetLabel}</span>
          <span><Activity size={14} />动作：{managedPresentation?.actionLabel}</span>
          <span><Clock3 size={14} />心跳：{managedPresentation?.heartbeatLabel}</span>
          <span className="managed-flow-detail">错误原因：{managedState.error_message || "暂无"}</span>
          {managedPresentation?.detail && managedPresentation.detail !== managedState.error_message && (
            <span className="managed-flow-detail">最近动作：{managedPresentation.detail}</span>
          )}
        </div>
        <div className="managed-flow-actions" style={{ display: "flex", gap: "8px", flexWrap: "wrap", marginTop: "8px" }}>
          {managedPresentation?.canPause && onPauseManagedFlow && (
            <button type="button" className="ap-bar-btn" disabled={busy} onClick={() => { void onPauseManagedFlow(); }}>
              <Pause size={14} /> 暂停托管
            </button>
          )}
          {managedPresentation?.canResume && onResumeManagedFlow && (
            <button type="button" className="ap-bar-btn ap-bar-btn-primary" disabled={busy} onClick={() => { void onResumeManagedFlow(); }}>
              <Play size={14} /> {managedPresentation.resumeLabel}
            </button>
          )}
          {managedActive && onStopManagedFlow && (
            <button type="button" className="ap-bar-btn" disabled={busy} onClick={() => { void onStopManagedFlow(); }}>
              <Square size={14} /> 停止托管并转人工
            </button>
          )}
          {managedPresentation?.nextStepLabel && (
            <span className="managed-flow-next-step">下一步：{managedPresentation.nextStepLabel}</span>
          )}
        </div>
      </div>
    </div>
  ) : null;

  if (["MilestoneGeneration", "MilestoneCheck", "MilestoneApproval", "MilestoneSelection"].includes(step)) {
    return <><NativeReadinessPanel readiness={project.native_readiness} />{managedBanner}<MilestonePlanningStep project={project} busy={planningBusy} feedback={feedback}
      regenerationFeedback={regenerationFeedback} setRegenerationFeedback={setRegenerationFeedback}
      regenerationModalOpen={milestoneModalOpen} setRegenerationModalOpen={setMilestoneModalOpen}
      onGenerate={handleGenerateMilestone}
      onCheck={() => runProjectCommand("check_milestone_draft", { projectName: project.name }, "大阶段检查已完成。")}
      onApprove={() => runProjectCommand("approve_milestone_draft", { projectName: project.name }, "大阶段已批准。")}
      onSelect={(milestoneId) => runProjectCommand("select_milestone", { projectName: project.name, milestoneId }, "已选择大阶段。")}
      onContinue={() => runProjectCommand("continue_current_milestone", { projectName: project.name }, "已按项目事实继续当前大阶段。")}
      onRegenerate={handleRegenerateMilestone}
    /></>;
  }

  if (["MidStageGeneration", "MidStageCheck", "MidStageApproval", "MidStageSelection"].includes(step)) {
    return <><NativeReadinessPanel readiness={project.native_readiness} />{managedBanner}<MidStagePlanningStep project={project} busy={planningBusy} feedback={feedback}
      regenerationFeedback={regenerationFeedback} setRegenerationFeedback={setRegenerationFeedback}
      regenerationModalOpen={midStageModalOpen} setRegenerationModalOpen={setMidStageModalOpen}
      onGenerate={() => runProjectCommand("generate_mid_stage_draft", { projectName: project.name }, "中阶段草稿已生成。")}
      onCheck={() => runProjectCommand("check_mid_stage_draft", { projectName: project.name }, "中阶段检查已完成。")}
      onApprove={() => runProjectCommand("approve_mid_stage_draft", { projectName: project.name }, "中阶段已批准。")}
      onSelect={(midStageId) => runProjectCommand("select_mid_stage", { projectName: project.name, midStageId }, "已选择中阶段。")}
      onContinue={() => runProjectCommand("select_mid_stage", { projectName: project.name, midStageId: project.current_mid_stage_id }, "已按中阶段事实继续。")}
      onRegenerate={handleRegenerateMidStage}
    /></>;
  }

  if (["PlanGeneration", "PlanCheck", "PlanApproving"].includes(step)) {
    return <><NativeReadinessPanel readiness={project.native_readiness} />{managedBanner}<ExecutionPlanStep project={project} busy={planningBusy} feedback={feedback}
      regenerationFeedback={regenerationFeedback} setRegenerationFeedback={setRegenerationFeedback}
      regenerationModalOpen={planModalOpen} setRegenerationModalOpen={setPlanModalOpen}
      onGenerate={() => runProjectCommand("generate_execution_plan", { projectName: project.name }, "执行计划已生成。")}
      onCheck={() => runProjectCommand("check_stage_plan", { projectName: project.name }, "执行计划检查已完成。")}
      onApprove={() => runProjectCommand("approve_stage_plan", { projectName: project.name }, "执行计划已冻结，已进入执行阶段。")}
      onRegenerate={handleRegeneratePlan}
      workspaceStatus={workspaceStatus}
      onPrepareWorkspace={onPrepareWorkspace}
    /></>;
  }

  return null;
}
