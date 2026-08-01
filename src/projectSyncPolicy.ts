import type {
  ProjectStateChangedEvent,
  RuntimeSnapshot,
  TaskControlDetailStatus,
} from "./types";

export const TASK_CONTROL_DETAIL_STALE_AFTER_MS = 45_000;
export const TASK_CONTROL_FALLBACK_INTERVAL_MS = 30_000;
export const TASK_CONTROL_MAX_SYNC_FAILURES = 3;

export type TaskControlFallbackReason =
  | "channel_reconnecting"
  | "runtime_disconnected"
  | "detail_unavailable"
  | "detail_stale"
  | "sync_failures";

export interface TaskControlFallbackDecision {
  active: boolean;
  reason: TaskControlFallbackReason | null;
}

interface TaskControlFallbackFacts {
  enabled: boolean;
  subscriptionStatus: "idle" | "connected" | "reconnecting";
  runtimeSyncStatus: "idle" | "syncing" | "synced" | "delayed" | "disconnected";
  detailStatus: TaskControlDetailStatus;
  detailUpdatedAt: string | null;
  consecutiveFailures: number;
  nowMs?: number;
  staleAfterMs?: number;
  maxSyncFailures?: number;
}

export function taskControlFallbackDecision({
  enabled,
  subscriptionStatus,
  runtimeSyncStatus,
  detailStatus,
  detailUpdatedAt,
  consecutiveFailures,
  nowMs = Date.now(),
  staleAfterMs = TASK_CONTROL_DETAIL_STALE_AFTER_MS,
  maxSyncFailures = TASK_CONTROL_MAX_SYNC_FAILURES,
}: TaskControlFallbackFacts): TaskControlFallbackDecision {
  if (!enabled) return { active: false, reason: null };
  if (runtimeSyncStatus === "disconnected") {
    return { active: true, reason: "runtime_disconnected" };
  }
  if (subscriptionStatus === "reconnecting") {
    return { active: true, reason: "channel_reconnecting" };
  }
  if (consecutiveFailures >= maxSyncFailures) {
    return { active: true, reason: "sync_failures" };
  }
  if (detailStatus === "unavailable") {
    return { active: true, reason: "detail_unavailable" };
  }
  if (detailStatus === "ready" && detailUpdatedAt) {
    const updatedAtMs = Date.parse(detailUpdatedAt);
    if (Number.isFinite(updatedAtMs) && nowMs - updatedAtMs >= staleAfterMs) {
      return { active: true, reason: "detail_stale" };
    }
  }
  return { active: false, reason: null };
}

export interface ProjectSyncCursor {
  projectName: string;
  processStartId: string;
  eventSequence: number;
}

export function shouldAcceptProjectStateEvent(
  cursor: ProjectSyncCursor,
  event: ProjectStateChangedEvent,
): boolean {
  if (event.project_name !== cursor.projectName) return false;
  return event.process_start_id !== cursor.processStartId
    || event.event_sequence > cursor.eventSequence;
}

export function advanceProjectSyncCursor(
  cursor: ProjectSyncCursor,
  processStartId: string,
  eventSequence: number,
): ProjectSyncCursor {
  if (processStartId !== cursor.processStartId) {
    return { ...cursor, processStartId, eventSequence };
  }
  return {
    ...cursor,
    eventSequence: Math.max(cursor.eventSequence, eventSequence),
  };
}

export function shouldAcceptRuntimeSnapshot(
  cursor: ProjectSyncCursor,
  snapshot: RuntimeSnapshot,
): boolean {
  if (snapshot.project.name !== cursor.projectName) return false;
  return snapshot.process_start_id !== cursor.processStartId
    || snapshot.event_sequence >= cursor.eventSequence;
}

export function mergePendingProjectEvent(
  current: ProjectStateChangedEvent | null,
  incoming: ProjectStateChangedEvent,
): ProjectStateChangedEvent {
  if (!current || current.process_start_id !== incoming.process_start_id) return incoming;
  return incoming.event_sequence >= current.event_sequence ? incoming : current;
}
