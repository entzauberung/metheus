import type {
  PipelineState,
  Project,
  RecoveryPresentation,
  ResourceObservationSummary,
  RuntimeSnapshot,
  Subtask,
} from "./types";

export type RuntimeSyncStatus = "idle" | "syncing" | "synced" | "delayed" | "disconnected";

export type RuntimeOutcomeState =
  | "unknown"
  | "idle"
  | "executing"
  | "recovering"
  | "validating"
  | "awaiting_confirmation"
  | "quality_blocked"
  | "completed"
  | "failed"
  | "waiting_human";

export type RuntimeOutcomeTone = "neutral" | "active" | "warning" | "error" | "success";
export type RuntimeFactState = "unknown" | "pending" | "passed" | "blocked" | "not_required";

export interface RuntimeOutcomePresentation {
  state: RuntimeOutcomeState;
  statusLabel: string;
  summary: string;
  tone: RuntimeOutcomeTone;
  execution: RuntimeFactState;
  quality: RuntimeFactState;
  acceptance: RuntimeFactState;
  confirmation: "unknown" | "required" | "confirmed" | "not_required";
  recoveryKind: RecoveryPresentation["kind"];
  syncStatus: RuntimeSyncStatus;
  syncFresh: boolean;
  writeAllowed: boolean;
  writeBlockedReason: string;
}

export interface RuntimeOutcomeInput {
  snapshot: Pick<RuntimeSnapshot, "project" | "pipeline_state" | "recovery_presentation" | "resource_observation"> | null;
  syncStatus?: RuntimeSyncStatus;
}

export function formatRuntimeMutationFeedback(
  actionMessage: string,
  outcome: RuntimeOutcomePresentation,
): string {
  const actionSummary = actionMessage.trim() || "后端未提供动作摘要";
  return `${actionSummary}；当前任务：${outcome.statusLabel}：${outcome.summary}`;
}

export function runtimeOutcomeFeedbackType(
  outcome: RuntimeOutcomePresentation,
): "success" | "warning" | "info" {
  if (outcome.state === "completed") return "success";
  if (outcome.tone === "error" || outcome.tone === "warning") return "warning";
  return "info";
}

function findSubtask(tasks: Subtask[], taskId: string): Subtask | undefined {
  for (const task of tasks) {
    if (task.id === taskId) return task;
    const child = findSubtask(task.child_tasks ?? [], taskId);
    if (child) return child;
  }
  return undefined;
}

function currentSubtask(project: Project, pipeline: PipelineState | null): Subtask | undefined {
  const taskId = project.execution_session?.subtask_id || pipeline?.current_subtask_id || "";
  if (!taskId) return undefined;
  for (const milestone of project.milestones) {
    const direct = findSubtask(milestone.subtasks ?? [], taskId);
    if (direct) return direct;
    for (const midStage of milestone.mid_stages ?? []) {
      const nested = findSubtask(midStage.subtasks ?? [], taskId);
      if (nested) return nested;
    }
  }
  return undefined;
}

function acceptanceFact(task: Subtask | undefined): RuntimeFactState {
  const criteria = task?.acceptance_criteria ?? [];
  const ledger = task?.acceptance_ledger ?? [];
  if (criteria.length === 0) return ledger.length === 0 ? "not_required" : "blocked";
  if (ledger.length === 0) return "pending";
  const mapped = criteria.map((criterion, index) => (
    ledger.filter(item => item.criterion_index === index + 1 && item.criterion === criterion)
  ));
  const malformedRow = ledger.some(item => {
    const index = item.criterion_index - 1;
    return index < 0 || index >= criteria.length || criteria[index] !== item.criterion;
  });
  if (malformedRow || mapped.some(rows => rows.length > 1)) return "blocked";
  if (ledger.length !== criteria.length || mapped.some(rows => rows.length !== 1)) return "pending";
  const rows = mapped.map(([item]) => item!);
  if (rows.some(item => item.status === "Unsatisfied" || item.status === "Contradictory")) {
    return "blocked";
  }
  if (rows.some(item => (
    item.status === "Unknown"
      || item.status === "AiProvisionallySatisfied"
      || item.status === "DeferredHumanReview"
  ))) {
    return "pending";
  }
  return rows.every(item => item.status === "Satisfied" || item.status === "AcceptedDeviation")
    ? "passed"
    : "pending";
}

function qualityFact(task: Subtask | undefined): RuntimeFactState {
  if (!task?.test_result) return "pending";
  return task.test_result.passed ? "passed" : "blocked";
}

function syncFacts(status: RuntimeSyncStatus, hasSnapshot: boolean) {
  const syncFresh = hasSnapshot && status === "synced";
  if (syncFresh) {
    return { syncFresh, writeAllowed: true, writeBlockedReason: "" };
  }
  const writeBlockedReason = status === "disconnected"
    ? "运行时同步已断开，请先同步项目状态"
    : status === "syncing"
      ? "运行时快照正在同步，写操作暂不可用"
      : status === "delayed"
        ? "运行时状态同步延迟，写操作暂不可用"
        : "运行时快照尚未完成对账，写操作暂不可用";
  return { syncFresh, writeAllowed: false, writeBlockedReason };
}

function basePresentation(
  syncStatus: RuntimeSyncStatus,
  hasSnapshot: boolean,
): RuntimeOutcomePresentation {
  const sync = syncFacts(syncStatus, hasSnapshot);
  return {
    state: "unknown",
    statusLabel: "事实不足",
    summary: "运行时事实尚未建立，暂不判断执行结果",
    tone: "neutral",
    execution: "unknown",
    quality: "unknown",
    acceptance: "unknown",
    confirmation: "unknown",
    recoveryKind: "None",
    syncStatus,
    syncFresh: sync.syncFresh,
    writeAllowed: sync.writeAllowed,
    writeBlockedReason: sync.writeBlockedReason,
  };
}

function recoveryState(recovery: RecoveryPresentation): RuntimeOutcomeState | null {
  if (recovery.kind === "None") return null;
  if (recovery.progress_status === "waiting_human" || recovery.kind === "HumanDecision") {
    return "waiting_human";
  }
  if (recovery.progress_status === "inactive") return null;
  if (["queued", "scheduled", "running", "warning", "stalled"].includes(recovery.progress_status ?? "")) {
    return "recovering";
  }
  if (recovery.kind === "ValidationRetry" && recovery.progress_status) return "validating";
  return recovery.severity === "Error" ? "failed" : "waiting_human";
}

function stateCopy(state: RuntimeOutcomeState, quality: RuntimeFactState, acceptance: RuntimeFactState) {
  switch (state) {
    case "idle":
      return { statusLabel: "等待执行", summary: "当前没有活跃执行，等待后端或用户启动", tone: "neutral" as const };
    case "executing":
      return { statusLabel: "执行中", summary: "执行正在进行，尚未形成最终质量结论", tone: "active" as const };
    case "recovering":
      return { statusLabel: "恢复中", summary: "恢复动作进行中，执行结果和验收仍未完成", tone: "warning" as const };
    case "validating":
      return { statusLabel: "验证中", summary: "执行结果已返回，质量和验收事实仍在核对", tone: "active" as const };
    case "awaiting_confirmation":
      return {
        statusLabel: "待确认",
        summary: quality === "passed" && acceptance !== "blocked"
          ? "执行和质量事实已记录，等待确认事务"
          : "执行已结束，但质量或验收事实尚未满足确认条件",
        tone: "warning" as const,
      };
    case "quality_blocked":
      return { statusLabel: "质量受阻", summary: "质量门禁或验收账本未满足，不能宣称完成", tone: "error" as const };
    case "completed":
      return { statusLabel: "已完成", summary: "执行、质量、验收和确认均已由后端事实收口", tone: "success" as const };
    case "failed":
      return { statusLabel: "执行失败", summary: "执行未形成成功完成态，等待后端或人工边界", tone: "error" as const };
    case "waiting_human":
      return { statusLabel: "等待人工", summary: "后台动作已停止，等待人工处理", tone: "error" as const };
    case "unknown":
      return { statusLabel: "事实不足", summary: "运行时事实尚未建立，暂不判断执行结果", tone: "neutral" as const };
  }
}

function formatResourceBytes(value: number | undefined): string {
  if (value === undefined) return "未记录";
  if (value >= 1024 * 1024 * 1024) return `${(value / (1024 * 1024 * 1024)).toFixed(1)} GiB`;
  if (value >= 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}

function resourceObservationCopy(observation?: ResourceObservationSummary): string {
  const state = observation?.state ?? "Unknown";
  const stateLabel = {
    Unknown: "未知（不能视为安全）",
    MeasuredSafe: "已测安全",
    Warning: "警告",
    HardStop: "硬停止",
    KilledSuspected: "疑似资源终止",
  }[state];
  const sourceLabel = {
    Unknown: "未知",
    Proc: "进程 RSS",
    Cgroup: "cgroup",
    InProcess: "进程内",
  }[observation?.source ?? "Unknown"];
  const details = [
    `来源：${sourceLabel}`,
    `余量：${formatResourceBytes(observation?.headroom_bytes)}`,
    `采样：${observation?.sampled_at || "未记录"}`,
  ];
  return `资源状态：${stateLabel}；${details.join("；")}`;
}

export function resolveRuntimeOutcomePresentation({
  snapshot,
  syncStatus = "synced",
}: RuntimeOutcomeInput): RuntimeOutcomePresentation {
  const result = basePresentation(syncStatus, snapshot !== null);
  if (!snapshot) return result;

  const { project, pipeline_state: pipeline, recovery_presentation: recovery } = snapshot;
  const task = currentSubtask(project, pipeline);
  const quality = qualityFact(task);
  const acceptance = acceptanceFact(task);
  const sessionStatus = project.execution_session?.status.toLowerCase() ?? "";
  const sessionFailed = ["session_lost", "execution_failed", "stop_failed"].includes(sessionStatus);
  const execution = sessionFailed || task?.execution_result?.success === false || pipeline?.status === "Failed"
    ? "blocked"
    : pipeline?.status === "Running" || ["executing", "recovering"].includes(sessionStatus)
      ? "pending"
      : task?.execution_result?.success === true
        ? "passed"
        : "unknown";
  const confirmationRequired = pipeline?.awaiting_confirmation === true
    || ["awaiting_confirmation", "confirming", "rejecting"].includes(sessionStatus)
    || task?.status === "AwaitingConfirmation";
  const confirmation = task?.status === "Passed" || task?.status === "AcceptedDeviation"
    ? "confirmed"
    : confirmationRequired
      ? "required"
      : "not_required";

  let state = recoveryState(recovery);
  if (!state && execution === "blocked") state = "failed";
  if (!state && project.workflow_state.managed_flow_state?.active) {
    state = project.workflow_state.managed_flow_state.run_status === "ErrorStopped"
      ? "waiting_human"
      : project.workflow_state.managed_flow_state.run_status === "Running"
        ? "executing"
        : "idle";
  }
  if (!state && (pipeline?.status === "Running" || ["executing", "recovering"].includes(sessionStatus))) {
    state = "executing";
  }
  if (!state && project.execution_session?.verification_stage
    && project.execution_session.verification_stage !== "NotStarted"
    && !confirmationRequired) {
    state = "validating";
  }
  if (!state && (pipeline?.status === "Failed" || sessionFailed || execution === "blocked")) state = "failed";
  if (!state && pipeline?.status === "Completed" && execution !== "passed") state = "unknown";
  if (!state && sessionStatus === "quality_blocked") state = "quality_blocked";
  if (!state && sessionStatus === "confirmation_blocked") state = "waiting_human";
  if (!state && confirmationRequired) {
    state = quality === "blocked" || acceptance === "blocked"
      ? "quality_blocked"
      : quality === "pending" || acceptance === "pending"
        ? "validating"
        : "awaiting_confirmation";
  }
  const taskCompleted = task?.status === "Passed" || task?.status === "AcceptedDeviation";
  if (!state && taskCompleted && execution === "passed" && quality === "passed"
    && ["passed", "not_required"].includes(acceptance) && confirmation === "confirmed") {
    state = "completed";
  }
  if (!state && (task?.status === "Rejected" || task?.status === "RolledBack")) state = "failed";
  // A workflow marker alone is not a completion proof. Keep the outcome
  // unknown when the execution, quality, acceptance, or confirmation facts
  // are missing from an otherwise completed-looking legacy snapshot.
  if (!state && project.workflow_state.current_step === "Completed") state = "unknown";
  if (!state && task?.status === "Executing") state = "executing";
  if (!state && (quality === "blocked" || acceptance === "blocked")) state = "quality_blocked";
  if (!state) state = "idle";

  const copy = stateCopy(state, quality, acceptance);
  const sync = syncFacts(syncStatus, true);
  return {
    ...result,
    state,
    statusLabel: copy.statusLabel,
    summary: `${copy.summary}；${resourceObservationCopy(snapshot.resource_observation)}`,
    tone: copy.tone,
    execution,
    quality,
    acceptance,
    confirmation,
    recoveryKind: recovery.kind,
    syncFresh: sync.syncFresh,
    writeAllowed: sync.writeAllowed,
    writeBlockedReason: sync.writeBlockedReason,
  };
}
