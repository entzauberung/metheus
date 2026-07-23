import { EngineHealth, PipelineState, Project } from "./types";

const BLOCKING_HEALTH_STATUSES = new Set<EngineHealth["status"]>([
  "NotInstalled",
  "Unauthenticated",
  "UnsupportedVersion",
  "Disabled",
]);

export function engineHealthBlocksExecution(health: EngineHealth | null): boolean {
  return health !== null && BLOCKING_HEALTH_STATUSES.has(health.status);
}

export function engineChangeBlockedReason(
  project: Project,
  pipeline: PipelineState | null,
): string | null {
  if (pipeline?.status === "Running") return "执行正在运行";
  const recovery = project.workflow_state.recovery_state;
  const waitingEngine = recovery?.phase === "WaitingEngine";
  if (project.execution_session?.active && !waitingEngine) return "存在活跃执行会话";
  if (recovery && ["Diagnosing", "Repairing", "Retesting"].includes(recovery.phase)) {
    return "错误恢复正在进行";
  }
  const autopilot = project.workflow_state.autopilot_state;
  if (autopilot?.active && autopilot.run_status === "Running" && !waitingEngine) return "自动驾驶正在推进";
  const managed = project.workflow_state.managed_flow_state;
  if (managed?.active && managed.run_status === "Running") return "托管流程正在推进";
  return null;
}
