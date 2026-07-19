import { useState } from "react";
import { Project } from "./types";
import { invokeWithTimeout, isInvokeTimeoutError } from "./utils/invokeWithTimeout";
import { ConsoleFeedback } from "./components/ConsoleStepShell";
import { MilestonePlanningStep } from "./console/MilestonePlanningStep";
import { MidStagePlanningStep } from "./console/MidStagePlanningStep";
import { ExecutionPlanStep } from "./console/ExecutionPlanStep";

interface Props {
  project: Project;
  onProjectUpdated: (project: Project) => void;
}

type RegenerationSource = "check_failed" | "approval_rejected";

export function ConsoleWorkflowPanel({ project, onProjectUpdated }: Props) {
  const step = project.workflow_state.current_step;
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<ConsoleFeedback | null>(null);
  const [syncRequired, setSyncRequired] = useState(false);
  const [regenerationFeedback, setRegenerationFeedback] = useState("");
  const [milestoneModalOpen, setMilestoneModalOpen] = useState(false);
  const [midStageModalOpen, setMidStageModalOpen] = useState(false);
  const [planModalOpen, setPlanModalOpen] = useState(false);

  const syncProject = async () => {
    const latest = await invokeWithTimeout<Project>("get_project", { projectName: project.name });
    onProjectUpdated(latest);
    return latest;
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

  const handleSync = async () => {
    if (busy) return;
    setBusy(true);
    setFeedback({ type: "info", message: "正在同步项目状态..." });
    try {
      await syncProject();
      setSyncRequired(false);
      setFeedback({ type: "success", message: "项目状态已同步。" });
    } catch (error) {
      setSyncRequired(true);
      setFeedback({ type: "error", message: "同步失败：" + String(error) });
    } finally {
      setBusy(false);
    }
  };

  const runProjectCommand = async (
    command: string,
    args: Record<string, unknown>,
    successMessage: string,
  ) => {
    if (busy) return;
    setBusy(true);
    setFeedback(null);
    try {
      const updated = await invokeWithTimeout<Project>(command, args);
      onProjectUpdated(updated);
      setFeedback({ type: "success", message: successMessage });
    } catch (error) {
      setFeedback({ type: "error", message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const handleTransition = (targetStep: string) => runProjectCommand(
    "transition_workflow",
    { projectName: project.name, targetStep, reason: "用户手动推进" },
    "已进入下一规划步骤。",
  );

  const handleToggleAutopilot = async (active: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      const updated = await invokeWithTimeout<Project>("toggle_autopilot", {
        projectName: project.name,
        active,
      });
      onProjectUpdated(updated);
      setFeedback({ type: active ? "info" : "success", message: active ? "自动驾驶已激活 — 将自动串联大阶段内部所有步骤，仅在大阶段边界停下。" : "自动驾驶已关闭。" });
    } catch (error) {
      setFeedback({ type: "error", message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const handleAutopilotPause = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const updated = await invokeWithTimeout<Project>("autopilot_pause", {
        projectName: project.name,
      });
      onProjectUpdated(updated);
      setFeedback({ type: "info", message: "自动驾驶已暂停。" });
    } catch (error) {
      setFeedback({ type: "error", message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const handleGenerateMilestone = async () => {
    if (busy) return;
    const startingRevision = project.workflow_state.data_revision;
    setBusy(true); setFeedback(null); setSyncRequired(false);
    try {
      const updated = await invokeWithTimeout<Project>("generate_milestone_draft", { projectName: project.name });
      onProjectUpdated(updated);
      setFeedback({ type: "success", message: "大阶段草稿已生成，请运行质量检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        setFeedback({ type: "info", message: "请求等待超时，正在同步后端最终状态。" });
        const done = await coordinate((latest) => latest.workflow_state.data_revision > startingRevision && latest.workflow_state.current_step === "MilestoneCheck");
        if (done) setFeedback({ type: "success", message: "已同步后端完成的大阶段草稿。" });
        else {
          setSyncRequired(true);
          setFeedback({ type: "info", message: "后端未完成，请稍后手动同步项目状态。" });
        }
      } else setFeedback({ type: "error", message: "生成失败：" + String(error) });
    } finally { setBusy(false); }
  };

  const handleRegenerateMilestone = async (source: RegenerationSource) => {
    const draft = project.milestone_draft;
    if (busy || !draft) return;
    const revision = project.workflow_state.data_revision;
    const draftId = draft.draft_id;
    setBusy(true); setFeedback(null);
    try {
      const updated = await invokeWithTimeout<Project>("regenerate_milestone_draft", {
        projectName: project.name, currentDraftId: draftId,
        expectedDataRevision: revision, feedback: regenerationFeedback, source,
      });
      onProjectUpdated(updated);
      setRegenerationFeedback(""); setMilestoneModalOpen(false);
      setFeedback({ type: "success", message: "大阶段草稿已重新生成，请重新检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        const done = await coordinate((latest) => latest.workflow_state.data_revision > revision && latest.milestone_draft?.draft_id !== draftId);
        if (done) { setMilestoneModalOpen(false); setFeedback({ type: "success", message: "已同步新大阶段草稿。" }); }
        else { setSyncRequired(true); setFeedback({ type: "info", message: "后端未完成，请稍后同步项目状态。" }); }
      } else setFeedback({ type: "error", message: "重新生成失败：" + String(error) });
    } finally { setBusy(false); }
  };

  const handleRegenerateMidStage = async (source: RegenerationSource) => {
    const draft = project.mid_stage_draft;
    if (busy || !draft) return;
    const revision = project.workflow_state.data_revision;
    const draftId = draft.draft_id;
    setBusy(true); setFeedback(null);
    try {
      const updated = await invokeWithTimeout<Project>("regenerate_mid_stage_draft", {
        projectName: project.name, currentDraftId: draftId,
        expectedDataRevision: revision, feedback: regenerationFeedback, source,
      });
      onProjectUpdated(updated);
      setRegenerationFeedback(""); setMidStageModalOpen(false);
      setFeedback({ type: "success", message: "中阶段草稿已重新生成，请重新检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        const done = await coordinate((latest) => latest.workflow_state.data_revision > revision && latest.mid_stage_draft?.draft_id !== draftId);
        if (done) { setMidStageModalOpen(false); setFeedback({ type: "success", message: "已同步新中阶段草稿。" }); }
        else setFeedback({ type: "info", message: "后端未完成，请稍后同步项目状态。" });
      } else setFeedback({ type: "error", message: "重新生成失败：" + String(error) });
    } finally { setBusy(false); }
  };

  const handleRegeneratePlan = async (source: RegenerationSource) => {
    const milestone = project.milestones.find((item) => item.id === project.current_milestone_id);
    const midStage = milestone?.mid_stages.find((item) => item.id === project.current_mid_stage_id);
    if (busy || !midStage) return;
    const revision = project.workflow_state.data_revision;
    const planRevision = midStage.plan_draft_revision;
    setBusy(true); setFeedback(null);
    try {
      const updated = await invokeWithTimeout<Project>("regenerate_execution_plan", {
        projectName: project.name, expectedDataRevision: revision,
        expectedPlanDraftRevision: planRevision, feedback: regenerationFeedback, source,
      });
      onProjectUpdated(updated);
      setRegenerationFeedback(""); setPlanModalOpen(false);
      setFeedback({ type: "success", message: "执行计划已重新生成，请重新检查。" });
    } catch (error) {
      if (isInvokeTimeoutError(error)) {
        const done = await coordinate((latest) => {
          const latestMilestone = latest.milestones.find((item) => item.id === latest.current_milestone_id);
          const latestMid = latestMilestone?.mid_stages.find((item) => item.id === latest.current_mid_stage_id);
          return latest.workflow_state.data_revision > revision && (latestMid?.plan_draft_revision ?? 0) > planRevision;
        });
        if (done) { setPlanModalOpen(false); setFeedback({ type: "success", message: "已同步新执行计划。" }); }
        else setFeedback({ type: "info", message: "后端未完成，请稍后同步项目状态。" });
      } else setFeedback({ type: "error", message: "重新生成失败：" + String(error) });
    } finally { setBusy(false); }
  };

  const autopilotActive = project.workflow_state.autopilot_active === true;
  const autopilotRunning = autopilotActive && project.workflow_state.autopilot_state?.run_status === "Running";
  const managedActive = project.workflow_state.managed_flow_state?.active === true;
  const managedRunning = managedActive && project.workflow_state.managed_flow_state?.run_status === "Running";
  const managedPaused = managedActive && project.workflow_state.managed_flow_state?.run_status === "Paused";

  // Managed flow banner (shown during any Console step when managed flow is active)
  const managedBanner = managedActive ? (
    <div className="feedback-banner" style={{ marginBottom: "12px", padding: "10px 14px", background: managedRunning ? "#f0e6ff" : "#fff8e1", border: `1px solid ${managedRunning ? "#6e40c9" : "#d4a72c"}`, borderRadius: "6px", fontSize: "13px" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <span>
          🤖 <strong>托管层{managedRunning ? "运行中" : "已暂停"}</strong>
          {" — "}{project.workflow_state.managed_flow_state?.last_action || "自动推进中..."}
        </span>
        {managedRunning && (
          <button className="action-btn secondary" style={{ fontSize: "12px", padding: "4px 10px" }}
            onClick={async () => {
              const updated = await invokeWithTimeout<Project>("pause_managed_flow", { projectName: project.name });
              onProjectUpdated(updated);
            }}>暂停托管</button>
        )}
        {managedPaused && (
          <button className="action-btn primary" style={{ fontSize: "12px", padding: "4px 10px" }}
            onClick={async () => {
              const updated = await invokeWithTimeout<Project>("resume_managed_flow", { projectName: project.name });
              onProjectUpdated(updated);
            }}>恢复托管</button>
        )}
      </div>
    </div>
  ) : null;

  if (["MilestoneGeneration", "MilestoneCheck", "MilestoneApproval", "MilestoneSelection", "FuturePlanApproval"].includes(step)) {
    return <>{managedBanner}<MilestonePlanningStep project={project} busy={busy} feedback={feedback} syncRequired={syncRequired}
      regenerationFeedback={regenerationFeedback} setRegenerationFeedback={setRegenerationFeedback}
      regenerationModalOpen={milestoneModalOpen} setRegenerationModalOpen={setMilestoneModalOpen}
      onGenerate={handleGenerateMilestone}
      onCheck={() => runProjectCommand("check_milestone_draft", { projectName: project.name }, "大阶段检查已完成。")}
      onApprove={() => runProjectCommand("approve_milestone_draft", { projectName: project.name }, "大阶段已批准。")}
      onApproveFuture={() => runProjectCommand("approve_future_milestones", { projectName: project.name }, "后续大阶段已批准。")}
      onSelect={(milestoneId) => runProjectCommand("select_milestone", { projectName: project.name, milestoneId }, "已选择大阶段。")}
      onContinue={() => handleTransition("MidStageGeneration")}
      onRegenerate={handleRegenerateMilestone} onSync={handleSync}
      onToggleAutopilot={handleToggleAutopilot}
      onAutopilotPause={handleAutopilotPause}
      autopilotActive={autopilotActive}
      autopilotRunning={autopilotRunning} /></>;
  }

  if (["MidStageGeneration", "MidStageCheck", "MidStageApproval", "MidStageSelection"].includes(step)) {
    return <>{managedBanner}<MidStagePlanningStep project={project} busy={busy || autopilotRunning} feedback={feedback}
      regenerationFeedback={regenerationFeedback} setRegenerationFeedback={setRegenerationFeedback}
      regenerationModalOpen={midStageModalOpen} setRegenerationModalOpen={setMidStageModalOpen}
      onGenerate={() => runProjectCommand("generate_mid_stage_draft", { projectName: project.name }, "中阶段草稿已生成。")}
      onCheck={() => runProjectCommand("check_mid_stage_draft", { projectName: project.name }, "中阶段检查已完成。")}
      onApprove={() => runProjectCommand("approve_mid_stage_draft", { projectName: project.name }, "中阶段已批准。")}
      onSelect={(midStageId) => runProjectCommand("select_mid_stage", { projectName: project.name, midStageId }, "已选择中阶段。")}
      onContinue={() => handleTransition("PlanGeneration")} onRegenerate={handleRegenerateMidStage}
      autopilotActive={autopilotActive}
      autopilotRunning={autopilotRunning}
      onAutopilotPause={handleAutopilotPause} /></>;
  }

  if (["PlanGeneration", "PlanCheck", "PlanApproving"].includes(step)) {
    return <>{managedBanner}<ExecutionPlanStep project={project} busy={busy || autopilotRunning} feedback={feedback}
      regenerationFeedback={regenerationFeedback} setRegenerationFeedback={setRegenerationFeedback}
      regenerationModalOpen={planModalOpen} setRegenerationModalOpen={setPlanModalOpen}
      onGenerate={() => runProjectCommand("generate_execution_plan", { projectName: project.name }, "执行计划已生成。")}
      onCheck={() => runProjectCommand("check_stage_plan", { projectName: project.name }, "执行计划检查已完成。")}
      onApprove={() => runProjectCommand("approve_stage_plan", { projectName: project.name }, "执行计划已冻结，已进入执行阶段。")}
      onRegenerate={handleRegeneratePlan}
      autopilotActive={autopilotActive}
      autopilotRunning={autopilotRunning}
      onAutopilotPause={handleAutopilotPause} /></>;
  }

  return null;
}
