import { useState } from "react";
import {
  Activity,
  Check,
  FileQuestion,
  GitBranch,
  ListTodo,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  ScanSearch,
  Square,
  TestTube2,
  X,
} from "lucide-react";
import { getGitConfirmationBlockPresentation, getQualityStatusPresentation } from "./autopilotPolicy";
import { ActionButton } from "./components/ActionButton";
import { FeedbackBanner } from "./components/FeedbackBanner";
import { Modal } from "./components/Modal";
import { WorkflowActionBar } from "./components/WorkflowActionBar";
import {
  findFirstLeafByStatus,
  findFirstRunnableLeaf,
  findProjectSubtaskById,
  isSubtaskLeaf,
  isSubtaskRunnableLeaf,
} from "./taskTreePolicy";
import type { ExecutionWorkspaceStatus, PipelineState, Project } from "./types";
import { getWorkspaceAction } from "./workspacePolicy";

// ============================================================
// V1 执行面板：单小阶段执行 + 人工确认
// ============================================================
export default function V1ExecutionPanel({
  project, executionStatus, workspaceStatus, busy: externalBusy,
  onPrepareWorkspace, onExecute, onConfirm, onReject, onRetry, onInStop, onEdStop, onSyncProject,
  onAcknowledgeRecovery,
  onRetryGitConfirmation,
}: {
  project: Project; executionStatus: PipelineState | null;
  workspaceStatus: ExecutionWorkspaceStatus | null;
  busy: boolean;
  onPrepareWorkspace: () => Promise<void>;
  onExecute: () => Promise<void>; onConfirm: () => Promise<void>;
  onReject: (reason: string) => Promise<void>;
  onRetry: () => Promise<void>;
  onInStop: () => Promise<void>; onEdStop: () => Promise<void>;
  onSyncProject: () => Promise<void>;
  onAcknowledgeRecovery?: () => Promise<void>;
  onRetryGitConfirmation?: () => Promise<void>;
}) {
  const [rejectReason, setRejectReason] = useState("");
  const [localBusy, setLocalBusy] = useState(false);
  const [showReject, setShowReject] = useState(false);
  const busy = externalBusy || localBusy;

  const ms = project.milestones.find(m => m.id === project.current_milestone_id);
  const mid = ms?.mid_stages.find(m => m.id === project.current_mid_stage_id);
  const planApproved = mid?.plan_approved_at != null && (mid?.plan_revision ?? 0) > 0;

  const activeTasks = mid ? mid.subtasks : (ms?.subtasks ?? []);
  const sessionTask = findProjectSubtaskById(project, project.execution_session?.subtask_id ?? "");
  const sessionLeaf = sessionTask && isSubtaskLeaf(sessionTask) ? sessionTask : null;
  const nextSubtask = sessionLeaf
    && isSubtaskRunnableLeaf(project, sessionLeaf)
    ? sessionLeaf
    : findFirstRunnableLeaf(project, activeTasks);
  const awaitingSubtask = sessionLeaf?.status === "AwaitingConfirmation"
    ? sessionLeaf
    : findFirstLeafByStatus(activeTasks, "AwaitingConfirmation");

  const isAwaiting = executionStatus?.awaiting_confirmation === true || awaitingSubtask != null;
  const isExecuting = executionStatus?.status === "Running";

  // 精确失败会话：优先显示失败面板，暂时隐藏普通执行和准备环境
  const failedSession = project.execution_session;
  const failedStatus = (failedSession?.status ?? "").toLowerCase();
  const hasExecutionFailure =
    failedStatus === "execution_failed"
    || failedStatus === "session_lost"
    || failedStatus === "stop_failed";
  const hasConfirmationBlock = failedStatus === "confirmation_blocked";
  const confirmationPresentation = getGitConfirmationBlockPresentation(
    failedSession?.confirmation_failure_kind,
  );
  const recoveryActive = project.workflow_state.recovery_state != null;

  const handlePrepareWorkspace = async () => {
    if (!project || busy) return;
    setLocalBusy(true);
    try {
      await onPrepareWorkspace();
    } finally {
      setLocalBusy(false);
    }
  };

  const handleConfirm = async () => {
    setLocalBusy(true);
    await onConfirm();
    setLocalBusy(false);
  };

  const handleReject = async () => {
    if (!rejectReason.trim()) return;
    setLocalBusy(true);
    await onReject(rejectReason.trim());
    setRejectReason("");
    setShowReject(false);
    setLocalBusy(false);
  };

  const handleRetry = async () => {
    setLocalBusy(true);
    await onRetry();
    setLocalBusy(false);
  };

  // 质量判定：判断当前待确认任务是否可以确认通过
  const execOk = awaitingSubtask?.execution_result?.success === true;
  const humanOverride = awaitingSubtask?.human_verification?.verification_kind === "HumanOverride"
    && Boolean(awaitingSubtask.human_verification.verification_reason.trim());
  const testOk = awaitingSubtask?.test_result?.passed === true || humanOverride;
  const canConfirm = execOk && testOk && isAwaiting;
  const qualityStatuses = awaitingSubtask
    ? getQualityStatusPresentation(
      awaitingSubtask.test_result,
      awaitingSubtask.acceptance_ledger ?? [],
    )
    : [];
  const failureReason = !canConfirm && isAwaiting
    ? (!execOk ? "执行未成功" : !testOk ? "核验未通过" : null)
    : null;

  const workspaceReady = workspaceStatus?.ready_for_new_execution === true;
  const workspaceAction = getWorkspaceAction(workspaceStatus);
  const managedTaskChanges = workspaceStatus?.has_managed_task_changes === true
    && workspaceStatus.has_external_changes === false;

  return (
    <div className="v1-execution-panel" style={{ padding: "24px" }}>
      <h2 className="execution-panel-title"><ListTodo size={20} />执行</h2>

      {hasConfirmationBlock && failedSession && (
        <div className="execution-failure-panel" style={{
          marginBottom: "20px", padding: "16px",
          background: "#fff8c5", borderRadius: "8px", border: "1px solid #d4a72c",
        }}>
          <div style={{ fontWeight: 600, fontSize: "14px", marginBottom: "8px", color: "#7d4e00" }}>
            Git 确认受阻
          </div>
          <div style={{ fontSize: "13px", color: "#24292f", marginBottom: "8px", overflowWrap: "anywhere" }}>
            <div>任务：{failedSession.subtask_title || failedSession.subtask_id}</div>
            <div style={{ marginTop: "6px" }}>代码与质量结果已保留，不需要恢复执行基线。</div>
            {failedSession.failure_message && (
              <div style={{ marginTop: "6px", whiteSpace: "pre-wrap" }}>
                受阻原因：{failedSession.failure_message}
              </div>
            )}
          </div>
          <WorkflowActionBar>
            {confirmationPresentation.canRetry && onRetryGitConfirmation && (
              <ActionButton
                icon={<GitBranch size={16} />}
                loading={busy}
                loadingLabel="确认中"
                onClick={onRetryGitConfirmation}
              >
                重新确认提交
              </ActionButton>
            )}
            <ActionButton icon={<RotateCcw size={16} />} disabled={busy} onClick={onSyncProject}>
              同步状态
            </ActionButton>
          </WorkflowActionBar>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            {confirmationPresentation.hint}
          </p>
        </div>
      )}

      {/* 执行失败专用面板：优先显示，隐藏普通执行/准备环境 */}
      {hasExecutionFailure && failedSession && (
        <div className="execution-failure-panel" style={{
          marginBottom: "20px", padding: "16px",
          background: "#ffebe9", borderRadius: "8px", border: "1px solid #cf222e",
        }}>
          <div style={{ fontWeight: 600, fontSize: "14px", marginBottom: "8px", color: "#cf222e" }}>
            {failedStatus === "session_lost" ? "执行中断（进程失联）"
              : failedStatus === "stop_failed" ? "暂停失败"
              : "执行失败"}
          </div>
          <div style={{ fontSize: "13px", color: "#24292f", marginBottom: "8px", overflowWrap: "anywhere" }}>
            <div>受影响任务：{failedSession.subtask_title || failedSession.subtask_id}</div>
            {failedSession.failure_message && (
              <div style={{ marginTop: "6px", whiteSpace: "pre-wrap" }}>
                失败原因：{failedSession.failure_message}
              </div>
            )}
            {failedSession.base_commit && (
              <div style={{ marginTop: "4px", color: "#656d76", fontFamily: "monospace", fontSize: "12px" }}>
                基线：{failedSession.base_commit.slice(0, 12)}
              </div>
            )}
          </div>
          <WorkflowActionBar>
            <ActionButton
              icon={<RotateCcw size={16} />}
              loading={busy}
              loadingLabel="恢复中"
              onClick={async () => {
                setLocalBusy(true);
                try {
                  if (onAcknowledgeRecovery) {
                    await onAcknowledgeRecovery();
                  } else {
                    await onRetry();
                  }
                } finally {
                  setLocalBusy(false);
                }
              }}
            >
              恢复执行基线
            </ActionButton>
            <ActionButton icon={<RotateCcw size={16} />} disabled={busy} onClick={onSyncProject}>
              同步状态
            </ActionButton>
          </WorkflowActionBar>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            恢复成功前不会显示“已恢复到安全状态”。请先完成基线恢复，再重新执行。
          </p>
        </div>
      )}

      {/* Workspace status banner — 失败会话期间隐藏准备环境 */}
      {!hasExecutionFailure && !hasConfirmationBlock && !recoveryActive && planApproved && workspaceStatus && !workspaceReady && (
        <FeedbackBanner
          type={managedTaskChanges ? "info" : "warning"}
          message={workspaceStatus.status_message}
          details={workspaceStatus.changes.map(change =>
            `${change.tracked ? `${change.index_status}${change.worktree_status}` : "??"} ${change.path}${change.managed ? "（当前任务）" : ""}`
          )}
        />
      )}

      {/* Workspace preparation is only valid before repository metadata exists. */}
      {!hasExecutionFailure && !hasConfirmationBlock && !recoveryActive && planApproved && workspaceAction === "prepare" && (
        <div style={{ marginBottom: "20px" }}>
          <ActionButton icon={<GitBranch size={16} />} loading={busy} loadingLabel="准备中"
            onClick={handlePrepareWorkspace}>准备执行环境</ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            执行小阶段前需要初始化 Git 仓库并创建首次提交。
          </p>
        </div>
      )}

      {!hasExecutionFailure && !hasConfirmationBlock && !recoveryActive && planApproved && workspaceStatus &&
        workspaceAction !== "none" && workspaceAction !== "prepare"
        && workspaceAction !== "managed_task_changes" && (
        <div style={{ marginBottom: "20px" }}>
          <ActionButton icon={<RefreshCw size={16} />} disabled={busy} onClick={onSyncProject}>
            刷新工作区
          </ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            {workspaceAction === "resolve_changes"
              ? "请先处理上方列出的工作区变更，再刷新状态。"
              : workspaceAction === "configure_identity"
                ? "请先配置 Git user.name 和 user.email，再刷新状态。"
                : "请修复项目路径后刷新状态。"}
          </p>
        </div>
      )}

      {/* Awaiting confirmation */}
      {!hasExecutionFailure && !hasConfirmationBlock && !recoveryActive && isAwaiting && awaitingSubtask && (
        <div style={{ marginBottom: "20px" }}>
          <div style={{ padding: "14px", background: "#ddf4ff", borderRadius: "8px", border: "1px solid #0969da", marginBottom: "16px" }}>
            <strong>待确认：{awaitingSubtask.title}</strong>
            <div style={{ fontSize: "13px", color: "#656d76", marginTop: "8px" }}>
              <div>目标：{awaitingSubtask.goal || awaitingSubtask.title}</div>
              {awaitingSubtask.execution_result && (
                <>
                  <div style={{ marginTop: "4px" }}>变更文件：{awaitingSubtask.execution_result.file_changes?.join(", ") || "无"}</div>
                  <div style={{ marginTop: "4px", maxHeight: "150px", overflowY: "auto", background: "#f6f8fa", padding: "8px", borderRadius: "4px", fontFamily: "monospace", fontSize: "11px" }}>
                    {awaitingSubtask.execution_result.output?.slice(-1000)}
                  </div>
                </>
              )}
              {awaitingSubtask.test_result && (
                <div style={{ marginTop: "6px", display: "grid", gap: "4px" }}>
                  {qualityStatuses.map(status => (
                    <div key={status.key} style={{
                      display: "flex", alignItems: "center", gap: "5px",
                      color: status.tone === "success" ? "#1a7f37"
                        : status.tone === "error" ? "#cf222e"
                          : status.tone === "warning" ? "#9a6700" : "#656d76",
                    }}>
                      {status.key === "automated-test" ? <TestTube2 size={14} />
                        : status.key === "code-review" ? <ScanSearch size={14} />
                          : status.key === "review-protocol" ? <Activity size={14} />
                            : <FileQuestion size={14} />}
                      {status.label}
                    </div>
                  ))}
                  {awaitingSubtask.test_result.suggestion && (
                    <div style={{ color: "#656d76" }}>建议：{awaitingSubtask.test_result.suggestion}</div>
                  )}
                </div>
              )}
              {awaitingSubtask.human_verification && (
                <div style={{ marginTop: "4px", color: "#1a7f37" }}>
                  人工核验：{awaitingSubtask.human_verification.verification_reason}
                </div>
              )}
              <div style={{ marginTop: "4px" }}>验收标准：{awaitingSubtask.acceptance_criteria?.join("；") || "（无）"}</div>
            </div>
          </div>
          <WorkflowActionBar>
            {canConfirm ? (
              <ActionButton icon={<Check size={16} />} loading={busy} loadingLabel="确认中" onClick={handleConfirm}>确认通过</ActionButton>
            ) : (
              <ActionButton icon={<RotateCcw size={16} />} loading={busy} loadingLabel="恢复中" onClick={handleRetry}>恢复基线并重试</ActionButton>
            )}
            <ActionButton icon={<X size={16} />} variant="danger" disabled={busy} onClick={() => setShowReject(true)}>发现问题</ActionButton>
          </WorkflowActionBar>
          {failureReason && (
            <div style={{ padding: "10px 14px", background: "#fff8c5", borderRadius: "6px", border: "1px solid #d4a72c", marginTop: "12px", fontSize: "13px", color: "#9a6700" }}>
              ⚠️ 质量门禁阻断：{failureReason}。请先恢复基线并重试，或驳回后人工处理。
            </div>
          )}
          <Modal isOpen={showReject} onClose={() => setShowReject(false)} title="驳回执行结果"
            description="请记录需要修正的问题。" isDanger lockClose={busy} isSubmitting={busy}
            actions={[
              { label: "取消", onClick: () => setShowReject(false), variant: "secondary", disabled: busy },
              { label: busy ? "提交中..." : "确认驳回", onClick: handleReject, variant: "danger", disabled: busy || !rejectReason.trim() },
            ]}>
            <textarea className="console-feedback-input" value={rejectReason} onChange={e => setRejectReason(e.target.value)} placeholder="请说明发现的问题" disabled={busy} />
          </Modal>
        </div>
      )}

      {/* Next pending subtask — only when workspace is ready and no failure session */}
      {!hasExecutionFailure && !hasConfirmationBlock && !recoveryActive && !isAwaiting && planApproved && workspaceReady && nextSubtask && (
        <div style={{ marginBottom: "20px" }}>
          <div style={subtaskCardStyle}>
            <strong>下一个任务：{nextSubtask.title}</strong>
            <div style={{ fontSize: "13px", color: "#656d76", marginTop: "4px" }}>
              目标：{nextSubtask.goal || nextSubtask.title}
            </div>
            <div style={{ fontSize: "12px", color: "#656d76", marginTop: "2px" }}>
              允许修改：{nextSubtask.allowed_file_paths?.join(", ") || "—"} |
              允许新建：{nextSubtask.new_file_paths?.join(", ") || "—"}
            </div>
            <div style={{ fontSize: "12px", color: "#656d76", marginTop: "2px" }}>
              验收标准：{nextSubtask.acceptance_criteria?.join("；") || "（无）"}
            </div>
          </div>
          <ActionButton icon={<Play size={16} />} loading={busy || isExecuting} loadingLabel={isExecuting ? "执行中" : "启动中"}
            onClick={async () => { setLocalBusy(true); await onExecute(); setLocalBusy(false); }}>执行当前小阶段</ActionButton>
          <p style={{ color: "#656d76", fontSize: "12px", marginTop: "8px" }}>
            一次只执行一个已批准小阶段。执行完成后需要人工确认结果。
          </p>
        </div>
      )}

      {/* Pause controls — only visible when execution is actively running */}
      {!hasConfirmationBlock && isExecuting && !isAwaiting && (
        <div style={{
          marginBottom: "20px", padding: "16px",
          background: "#fff8f0", borderRadius: "8px", border: "1px solid #e6a23c",
        }}>
          <div style={{ fontWeight: 600, fontSize: "14px", marginBottom: "12px", color: "#9a6700" }}>
            ⏸ 暂停执行
          </div>
          <div style={{ display: "flex", gap: "16px", flexWrap: "wrap" }}>
            <div style={{ flex: 1, minWidth: "180px" }}>
              <ActionButton
                icon={<Square size={16} />}
                variant="danger"
                disabled={busy}
                onClick={async () => { setLocalBusy(true); await onInStop(); setLocalBusy(false); }}
                fullWidth
              >
                立即暂停 (In Stop)
              </ActionButton>
              <p style={{ color: "#656d76", fontSize: "12px", marginTop: "4px" }}>
                立即终止当前任务，回到上一个稳定检查点。未完成的任务不保留部分结果。
              </p>
            </div>
            <div style={{ flex: 1, minWidth: "180px" }}>
              <ActionButton
                icon={<Pause size={16} />}
                variant="secondary"
                disabled={busy}
                onClick={async () => { setLocalBusy(true); await onEdStop(); setLocalBusy(false); }}
                fullWidth
              >
                完成后暂停 (ED Stop)
              </ActionButton>
              <p style={{ color: "#656d76", fontSize: "12px", marginTop: "4px" }}>
                当前任务执行完成并确认后再暂停，已完成的任务得到保留。
              </p>
            </div>
          </div>
        </div>
      )}

      {/* All done — workflow should have auto-advanced; this is a safety net */}
      {!hasConfirmationBlock && !isAwaiting && planApproved && workspaceReady && !nextSubtask && (
        <div style={{ marginBottom: "20px" }}>
          <FeedbackBanner type="success" message="当前中阶段所有小阶段已执行完成。" />
          <p style={{ color: "#656d76", fontSize: "13px", marginTop: "12px" }}>
            如果页面未自动跳转，请手动同步项目状态。
          </p>
          <ActionButton
            icon={<RotateCcw size={16} />}
            variant="secondary"
            onClick={onSyncProject}
          >
            同步项目状态
          </ActionButton>
        </div>
      )}

      {/* Execution log */}
      {executionStatus && (
        <div style={{ marginTop: "20px", padding: "10px", background: "#f6f8fa", borderRadius: "6px", fontSize: "12px", fontFamily: "monospace", color: "#656d76" }}>
          {executionStatus.current_log}
        </div>
      )}
    </div>
  );
}

const subtaskCardStyle: React.CSSProperties = {
  padding: "14px", background: "#f6f8fa", borderRadius: "8px",
  border: "1px solid #d0d7de", marginBottom: "12px",
};
