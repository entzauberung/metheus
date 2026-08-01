import type { PipelineState, RuntimeSnapshot } from "./types";

export type ExecutionPollDecision = "continue" | "reconcile";

export function executionPollDecision(
  status: PipelineState | null,
  projectName: string,
): ExecutionPollDecision {
  if (!status) return "reconcile";
  if (status.project_name && status.project_name !== projectName) return "reconcile";
  if (status.awaiting_confirmation) return "reconcile";
  return status.status === "Running" ? "continue" : "reconcile";
}

export type TerminalSyncPhase = "idle" | "terminal_reconciling" | "terminal_delayed";

const TERMINAL_SYNC_BACKOFF_MS = [0, 250, 750] as const;
const TERMINAL_DELAYED_BACKOFF_MS = [1_500, 3_000, 5_000] as const;

export function terminalSyncDelay(attempt: number): number | null {
  return TERMINAL_SYNC_BACKOFF_MS[attempt] ?? null;
}

export function terminalDelayedSyncDelay(attempt: number): number | null {
  return TERMINAL_DELAYED_BACKOFF_MS[attempt] ?? null;
}

export const TERMINAL_SYNC_MAX_WAIT_MS = TERMINAL_SYNC_BACKOFF_MS.reduce<number>(
  (sum, value) => sum + value,
  0,
) + TERMINAL_DELAYED_BACKOFF_MS.reduce<number>((sum, value) => sum + value, 0);

export function shouldReconcileAfterPollFailure(consecutiveFailures: number): boolean {
  return consecutiveFailures >= 2;
}

const ACTIVE_SESSION_STATUSES = new Set([
  "executing",
  "recovering",
  "replanning",
  "confirming",
  "rejecting",
]);

/** A terminal poll is reconciled only after both runtime and durable session facts agree. */
export function isTerminalRuntimeSnapshot(
  snapshot: RuntimeSnapshot | null,
  projectName: string,
): snapshot is RuntimeSnapshot {
  if (!snapshot || snapshot.project.name !== projectName) return false;
  if (snapshot.pipeline_state?.status === "Running") return false;
  const session = snapshot.project.execution_session;
  if (!session) return true;
  return !(session.active && ACTIVE_SESSION_STATUSES.has(session.status.toLowerCase()));
}
