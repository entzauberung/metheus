import type {
  ProjectStateChangedEvent,
  RecoveryPresentation,
  RuntimeSnapshot,
  TaskControlDetailStatus,
} from "./types";

export const TASK_CONTROL_DETAIL_STALE_AFTER_MS = 45_000;
export const TASK_CONTROL_FALLBACK_INTERVAL_MS = 30_000;
export const TASK_CONTROL_MAX_SYNC_FAILURES = 3;
export const PROJECT_SYNC_CONNECTED_FALLBACK_MS = 60_000;
export const PROJECT_SYNC_ACTIVE_RECOVERY_FALLBACK_MS = 5_000;
export const PROJECT_SYNC_DISCONNECTED_FALLBACK_MS = 15_000;
export const PROJECT_SYNC_FORCE_BACKOFF_MS = 1_000;

export function shouldStartForcedSync(
  lastForcedSyncAtMs: number | null,
  nowMs = Date.now(),
  backoffMs = PROJECT_SYNC_FORCE_BACKOFF_MS,
): boolean {
  return lastForcedSyncAtMs === null || nowMs - lastForcedSyncAtMs >= backoffMs;
}

export function recoveryPresentationIsActive(
  presentation: RecoveryPresentation,
): boolean {
  switch (presentation.progress_status) {
    case "queued":
    case "scheduled":
    case "running":
    case "warning":
    case "stalled":
      return true;
    case "inactive":
    case "waiting_human":
      return false;
    default:
      return presentation.background_retry_active === true;
  }
}

export function projectSyncFallbackIntervalMs(
  subscriptionStatus: "idle" | "connected" | "reconnecting",
  runtimeSyncStatus: "idle" | "syncing" | "synced" | "delayed" | "disconnected",
  connectedIntervalMs = PROJECT_SYNC_CONNECTED_FALLBACK_MS,
  activeRecovery = false,
): number {
  if (subscriptionStatus === "connected" && runtimeSyncStatus !== "disconnected") {
    return activeRecovery
      ? Math.min(connectedIntervalMs, PROJECT_SYNC_ACTIVE_RECOVERY_FALLBACK_MS)
      : connectedIntervalMs;
  }
  return Math.min(connectedIntervalMs, PROJECT_SYNC_DISCONNECTED_FALLBACK_MS);
}

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
  if (detailStatus === "ready") {
    if (!detailUpdatedAt) return { active: true, reason: "detail_stale" };
    const updatedAtMs = Date.parse(detailUpdatedAt);
    if (!Number.isFinite(updatedAtMs) || nowMs - updatedAtMs >= staleAfterMs) {
      return { active: true, reason: "detail_stale" };
    }
  }
  return { active: false, reason: null };
}

export interface ProjectSyncCursor {
  projectName: string;
  processStartId: string;
  eventSequence: number;
  dataRevision: number;
  taskControlTreeRevision: number;
  taskControlSnapshotVersion: string;
  taskControlActionId: string | null;
  taskControlMode: ProjectStateChangedEvent["control_mode"] | null;
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
    return {
      ...cursor,
      processStartId,
      eventSequence,
      dataRevision: 0,
      taskControlTreeRevision: 0,
      taskControlSnapshotVersion: "",
      taskControlActionId: null,
      taskControlMode: null,
    };
  }
  return {
    ...cursor,
    eventSequence: Math.max(cursor.eventSequence, eventSequence),
  };
}

export function shouldRequestRuntimeSnapshot(
  cursor: ProjectSyncCursor,
  event: ProjectStateChangedEvent,
): boolean {
  if (!shouldAcceptProjectStateEvent(cursor, event)) return false;
  if (event.process_start_id !== cursor.processStartId) return true;
  if (event.runtime_dirty) return true;
  if (event.data_revision > cursor.dataRevision) return true;
  return event.task_control_dirty && (
    event.task_control_tree_revision > cursor.taskControlTreeRevision
    || event.task_control_snapshot_version !== cursor.taskControlSnapshotVersion
    || event.control_action_id !== cursor.taskControlActionId
    || event.control_mode !== cursor.taskControlMode
  );
}

export function advanceProjectSyncRevisions(
  cursor: ProjectSyncCursor,
  dataRevision: number,
  taskControlTreeRevision: number,
  taskControlSnapshotVersion: string,
  taskControlActionId: string | null,
  taskControlMode: ProjectSyncCursor["taskControlMode"],
): ProjectSyncCursor {
  return {
    ...cursor,
    dataRevision: Math.max(cursor.dataRevision, dataRevision),
    taskControlTreeRevision: Math.max(
      cursor.taskControlTreeRevision,
      taskControlTreeRevision,
    ),
    taskControlSnapshotVersion,
    taskControlActionId,
    taskControlMode,
  };
}

export function shouldAcceptRuntimeSnapshot(
  cursor: ProjectSyncCursor,
  snapshot: RuntimeSnapshot,
): boolean {
  if (snapshot.project.name !== cursor.projectName) return false;
  if (snapshot.process_start_id !== cursor.processStartId) return true;
  if (snapshot.event_sequence > cursor.eventSequence) return true;
  if (snapshot.event_sequence < cursor.eventSequence) return false;

  const snapshotDataRevision = snapshot.project.workflow_state?.data_revision;
  if (snapshotDataRevision !== undefined && snapshotDataRevision < cursor.dataRevision) {
    return false;
  }
  if (snapshot.task_control_tree_revision < cursor.taskControlTreeRevision) {
    return false;
  }
  if (snapshot.event_sequence === cursor.eventSequence
    && snapshot.task_control_tree_revision === cursor.taskControlTreeRevision
    && cursor.taskControlSnapshotVersion
    && snapshot.task_control_snapshot_version !== cursor.taskControlSnapshotVersion) {
    return false;
  }
  if (snapshot.event_sequence === cursor.eventSequence
    && snapshot.task_control_tree_revision === cursor.taskControlTreeRevision
    && cursor.taskControlActionId !== null
    && snapshot.task_control_action_id !== cursor.taskControlActionId) {
    return false;
  }
  return true;
}

export function mergePendingProjectEvent(
  current: ProjectStateChangedEvent | null,
  incoming: ProjectStateChangedEvent,
): ProjectStateChangedEvent {
  if (!current || current.process_start_id !== incoming.process_start_id) return incoming;
  return incoming.event_sequence >= current.event_sequence ? incoming : current;
}
