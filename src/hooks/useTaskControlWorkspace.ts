import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { findProjectSubtaskById, findTaskControlNode } from "../taskTreePolicy";
import {
  taskControlFallbackDecision,
  TASK_CONTROL_DETAIL_STALE_AFTER_MS,
  TASK_CONTROL_FALLBACK_INTERVAL_MS,
  TASK_CONTROL_MAX_SYNC_FAILURES,
  type TaskControlFallbackReason,
} from "../projectSyncPolicy";
import {
  createFollowingTaskSelection,
  createPinnedTaskSelection,
  reconcileTaskSelection,
  type TaskSelectionMode,
  type TaskSelectionState,
} from "../taskSelectionPolicy";
import type {
  Project,
  RuntimeMutationResult,
  Subtask,
  TaskControlDetailStatus,
  TaskControlMode,
  TaskControlSnapshot,
  TaskTreeNodeView,
} from "../types";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";

const DETAIL_RETRY_DELAYS_MS = [1_000, 3_000] as const;

interface TaskControlActionOptions {
  criterionIndexes?: number[];
  reason?: string;
}

interface UseTaskControlWorkspaceOptions {
  project: Project | null;
  enabled?: boolean;
  pollIntervalMs?: number;
  invalidationSequence?: number;
  runtimeCursor?: TaskControlRuntimeCursor;
  atomicSnapshot?: TaskControlSnapshot | null;
  atomicSnapshotStatus?: TaskControlDetailStatus;
  atomicSnapshotUpdatedAt?: string | null;
  subscriptionStatus?: "idle" | "connected" | "reconnecting";
  runtimeSyncStatus?: "idle" | "syncing" | "synced" | "delayed" | "disconnected";
  runtimeSyncFailures?: number;
  detailStaleAfterMs?: number;
  maxSyncFailures?: number;
  onRuntimeMutation?: (result: RuntimeMutationResult) => unknown;
}

export interface TaskControlWorkspace {
  snapshot: TaskControlSnapshot | null;
  selectedTaskId: string;
  selectionMode: TaskSelectionMode;
  selectedNode: TaskTreeNodeView | null;
  busy: boolean;
  error: string;
  sourceEventSequence: number;
  detailsSyncing: boolean;
  detailFallbackActive: boolean;
  detailFallbackReason: TaskControlFallbackReason | null;
  refresh: () => Promise<void>;
  selectTask: (taskId: string) => void;
  followCurrentTask: () => void;
  executeAction: (name: string, options?: TaskControlActionOptions) => Promise<void>;
  changeMode: (mode: TaskControlMode, reason?: string) => Promise<void>;
}

export interface TaskControlRuntimeCursor {
  processStartId: string;
  eventSequence: number;
  projectRevision: number;
  treeRevision: number;
  controlActionId: string | null;
  controlActionKnown?: boolean;
  snapshotVersion: string;
}

export function isTaskControlSnapshotCurrent(
  snapshot: TaskControlSnapshot,
  cursor: TaskControlRuntimeCursor,
): boolean {
  if (cursor.processStartId && snapshot.source_process_start_id !== cursor.processStartId) {
    return false;
  }
  if (snapshot.source_event_sequence < cursor.eventSequence
    || snapshot.project_revision < cursor.projectRevision
    || snapshot.task_tree_revision < cursor.treeRevision) {
    return false;
  }
  if (cursor.snapshotVersion && snapshot.snapshot_version !== cursor.snapshotVersion) {
    return false;
  }
  return !cursor.controlActionKnown
    || snapshot.source_control_action_id === cursor.controlActionId;
}

function subtaskView(task: Subtask): TaskTreeNodeView {
  const contract = task.contract_snapshot;
  return {
    id: task.id,
    title: task.title,
    node_type: "Subtask",
    status: task.status,
    depth: contract?.depth ?? 0,
    complexity: contract?.complexity ?? "Small",
    risk: contract?.risk ?? "Low",
    contract_fingerprint: contract?.fingerprint ?? "",
    contract,
    dependencies: task.depends_on ?? [],
    acceptance: task.acceptance_ledger ?? [],
    capabilities: [],
    disabled_reasons: {
      execute: "后端节点能力尚未同步，当前任务只读",
      revalidate: "后端节点能力尚未同步，当前任务只读",
      split: "后端节点能力尚未同步，当前任务只读",
      recompile: "后端节点能力尚未同步，当前任务只读",
      accept_deviation: "后端节点能力尚未同步，当前任务只读",
    },
    is_currently_actionable: false,
    actionable_acceptance_criteria: [],
    children: (task.child_tasks ?? []).map(subtaskView),
  };
}

function taskExists(
  snapshot: TaskControlSnapshot,
  project: Project,
  taskId: string,
): boolean {
  return Boolean(taskId && (
    findTaskControlNode(snapshot.nodes, taskId)
    || findProjectSubtaskById(project, taskId)
  ));
}

export function useTaskControlWorkspace({
  project,
  enabled = true,
  pollIntervalMs = TASK_CONTROL_FALLBACK_INTERVAL_MS,
  invalidationSequence = 0,
  runtimeCursor,
  atomicSnapshot = null,
  atomicSnapshotStatus = "unavailable",
  atomicSnapshotUpdatedAt = null,
  subscriptionStatus = "connected",
  runtimeSyncStatus = "synced",
  runtimeSyncFailures = 0,
  detailStaleAfterMs = TASK_CONTROL_DETAIL_STALE_AFTER_MS,
  maxSyncFailures = TASK_CONTROL_MAX_SYNC_FAILURES,
  onRuntimeMutation,
}: UseTaskControlWorkspaceOptions): TaskControlWorkspace {
  const [snapshot, setSnapshot] = useState<TaskControlSnapshot | null>(null);
  const [selection, setSelection] = useState<TaskSelectionState>(
    () => createFollowingTaskSelection(),
  );
  const selectedTaskId = selection.selectedTaskId;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [sourceEventSequence, setSourceEventSequence] = useState(0);
  const [detailRetryAttempt, setDetailRetryAttempt] = useState(0);
  const [fallbackClock, setFallbackClock] = useState(() => Date.now());
  const invalidationSequenceRef = useRef(invalidationSequence);
  invalidationSequenceRef.current = invalidationSequence;
  const runtimeCursorRef = useRef<TaskControlRuntimeCursor | undefined>(runtimeCursor);
  runtimeCursorRef.current = runtimeCursor;
  const requestSequence = useRef(0);
  const projectRef = useRef(project);
  projectRef.current = project;
  const projectName = project?.name ?? "";
  const projectRevision = project?.workflow_state.data_revision ?? 0;
  const runtimeCursorKey = runtimeCursor
    ? [
      runtimeCursor.processStartId,
      runtimeCursor.eventSequence,
      runtimeCursor.projectRevision,
      runtimeCursor.treeRevision,
      runtimeCursor.controlActionId ?? "",
      runtimeCursor.snapshotVersion,
    ].join(":")
    : "";
  const scope = useRef({
    projectName,
    projectRevision,
  });
  scope.current = {
    projectName,
    projectRevision,
  };

  const applyDetailedSnapshot = useCallback((
    next: TaskControlSnapshot,
    currentProject: Project,
  ) => {
    setSnapshot(current => (
      current && current.project_name === next.project_name
        && current.project_revision > next.project_revision
        ? current
        : next
    ));
    setSourceEventSequence(current => Math.max(
      current,
      next.source_event_sequence,
    ));
    setSelection(current => reconcileTaskSelection(
      current,
      next.current_task_id,
      {
        currentTaskExists: taskExists(next, currentProject, next.current_task_id),
        selectedTaskExists: taskExists(next, currentProject, current.selectedTaskId),
      },
    ));
    setDetailRetryAttempt(0);
    setError("");
  }, []);

  const refresh = useCallback(async () => {
    const currentProject = projectRef.current;
    if (!enabled || !currentProject?.name) return;
    const requestedProjectName = currentProject.name;
    const sequence = ++requestSequence.current;
    const requestedRevision = currentProject.workflow_state.data_revision;
    const requestedEventSequence = invalidationSequenceRef.current;
    const expectedCursor = runtimeCursorRef.current ?? {
      processStartId: "",
      eventSequence: requestedEventSequence,
      projectRevision: requestedRevision,
      treeRevision: 0,
      controlActionId: null,
      controlActionKnown: false,
      snapshotVersion: "",
    };
    try {
      const next = await invokeWithTimeout<TaskControlSnapshot>("get_task_control_snapshot", {
        projectName: requestedProjectName,
      }, 10_000);
      const currentScope = scope.current;
      if (
        sequence !== requestSequence.current
        || currentScope.projectName !== requestedProjectName
        || next.project_name !== requestedProjectName
        || next.project_revision < requestedRevision
        || next.project_revision < currentScope.projectRevision
        || !isTaskControlSnapshotCurrent(next, expectedCursor)
      ) {
        return;
      }
      applyDetailedSnapshot(next, currentProject);
    } catch (reason) {
      if (sequence === requestSequence.current && scope.current.projectName === requestedProjectName) {
        setError(String(reason));
        setDetailRetryAttempt(current => Math.min(
          current + 1,
          DETAIL_RETRY_DELAYS_MS.length + 1,
        ));
      }
    }
  }, [applyDetailedSnapshot, enabled]);

  useEffect(() => {
    requestSequence.current += 1;
    setSnapshot(null);
    setSelection(createFollowingTaskSelection());
    setSourceEventSequence(0);
    setDetailRetryAttempt(0);
    setError("");
  }, [projectName]);

  useEffect(() => {
    setDetailRetryAttempt(0);
  }, [invalidationSequence, runtimeCursorKey]);

  const atomicSnapshotIsCurrent = !!atomicSnapshot
    && atomicSnapshot.project_name === projectName
    && (!runtimeCursor || isTaskControlSnapshotCurrent(atomicSnapshot, runtimeCursor));
  const waitingForAtomicSnapshot = atomicSnapshotStatus === "idle"
    || atomicSnapshotStatus === "syncing";
  const fallbackDecision = taskControlFallbackDecision({
    enabled: enabled && Boolean(projectName),
    subscriptionStatus,
    runtimeSyncStatus,
    detailStatus: atomicSnapshotStatus,
    detailUpdatedAt: atomicSnapshotUpdatedAt,
    consecutiveFailures: runtimeSyncFailures,
    nowMs: fallbackClock,
    staleAfterMs: detailStaleAfterMs,
    maxSyncFailures,
  });

  useEffect(() => {
    if (!enabled || !projectName || atomicSnapshotStatus !== "ready" || !atomicSnapshotUpdatedAt) {
      return;
    }
    const updatedAtMs = Date.parse(atomicSnapshotUpdatedAt);
    if (!Number.isFinite(updatedAtMs)) return;
    const remaining = detailStaleAfterMs - (Date.now() - updatedAtMs);
    if (remaining <= 0) {
      setFallbackClock(Date.now());
      return;
    }
    const timer = window.setTimeout(() => setFallbackClock(Date.now()), remaining);
    return () => window.clearTimeout(timer);
  }, [atomicSnapshotStatus, atomicSnapshotUpdatedAt, detailStaleAfterMs, enabled, projectName]);

  useEffect(() => {
    const currentProject = projectRef.current;
    if (!enabled || !currentProject || !atomicSnapshotIsCurrent || !atomicSnapshot) return;
    requestSequence.current += 1;
    applyDetailedSnapshot(atomicSnapshot, currentProject);
  }, [applyDetailedSnapshot, atomicSnapshot, atomicSnapshotIsCurrent, enabled]);

  useEffect(() => {
    if (!enabled || !project?.name) return;
    if (atomicSnapshotIsCurrent) return;
    if (waitingForAtomicSnapshot) return;
    if (!fallbackDecision.active) return;
    void refresh();
  }, [atomicSnapshotIsCurrent, enabled, fallbackDecision.active, fallbackDecision.reason, invalidationSequence, projectName, refresh, runtimeCursorKey, waitingForAtomicSnapshot]);

  useEffect(() => {
    if (!enabled || !projectName || !error || detailRetryAttempt === 0) return;
    const delay = DETAIL_RETRY_DELAYS_MS[detailRetryAttempt - 1];
    if (delay === undefined) return;
    const timer = window.setTimeout(() => { void refresh(); }, delay);
    return () => window.clearTimeout(timer);
  }, [detailRetryAttempt, enabled, error, projectName, refresh]);

  useEffect(() => {
    if (!enabled || !projectName || !fallbackDecision.active) return;
    const timer = window.setInterval(() => { void refresh(); }, pollIntervalMs);
    return () => window.clearInterval(timer);
  }, [enabled, fallbackDecision.active, fallbackDecision.reason, pollIntervalMs, projectName, refresh]);

  const snapshotIsCurrent = !!snapshot
    && (!runtimeCursor || isTaskControlSnapshotCurrent(snapshot, runtimeCursor));
  const visibleSnapshot = snapshotIsCurrent ? snapshot : null;

  const selectedNode = useMemo(() => {
    const fromSnapshot = visibleSnapshot
      ? findTaskControlNode(visibleSnapshot.nodes, selectedTaskId)
      : null;
    if (fromSnapshot) return fromSnapshot;
    const fromProject = project
      ? findProjectSubtaskById(project, selectedTaskId)
      : null;
    return fromProject ? subtaskView(fromProject) : null;
  }, [project, selectedTaskId, visibleSnapshot]);

  const selectTask = useCallback((taskId: string) => {
    setSelection(createPinnedTaskSelection(taskId));
  }, []);

  const followCurrentTask = useCallback(() => {
    const currentProject = projectRef.current;
    const currentTaskId = visibleSnapshot?.current_task_id ?? "";
    setSelection(createFollowingTaskSelection(
      visibleSnapshot && currentProject && taskExists(visibleSnapshot, currentProject, currentTaskId)
        ? currentTaskId
        : "",
    ));
  }, [visibleSnapshot]);

  const executeAction = useCallback(async (
    name: string,
    options: TaskControlActionOptions = {},
  ) => {
    if (!project?.name || !visibleSnapshot || busy) return;
    setBusy(true);
    setError("");
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>("apply_task_control_action_runtime", {
        projectName: project.name,
        request: {
          action: name,
          expected_revision: visibleSnapshot.project_revision,
          expected_tree_revision: visibleSnapshot.task_tree_revision,
          task_id: selectedTaskId || visibleSnapshot.current_task_id || undefined,
          decision_id: visibleSnapshot.decision?.decision_id ?? "",
          criterion_indexes: options.criterionIndexes ?? [],
          reason: options.reason ?? "",
        },
      }, 900_000);
      if (scope.current.projectName !== project.name) return;
      requestSequence.current += 1;
      if (result.task_control_snapshot) {
        const resultCursor: TaskControlRuntimeCursor = {
          processStartId: result.runtime_snapshot.process_start_id,
          eventSequence: result.runtime_snapshot.task_control_event_sequence,
          projectRevision: result.runtime_snapshot.project.workflow_state.data_revision,
          treeRevision: result.runtime_snapshot.task_control_tree_revision,
          controlActionId: result.runtime_snapshot.task_control_action_id,
          controlActionKnown: true,
          snapshotVersion: result.runtime_snapshot.task_control_snapshot_version,
        };
        if (isTaskControlSnapshotCurrent(result.task_control_snapshot, resultCursor)) {
          applyDetailedSnapshot(result.task_control_snapshot, project);
        }
      }
      onRuntimeMutation?.(result);
      if (!result.task_control_snapshot || !result.task_control.available) await refresh();
    } catch (reason) {
      if (scope.current.projectName === project.name) {
        setError(String(reason));
        if (String(reason).includes("修订冲突")) await refresh();
      }
    } finally {
      if (scope.current.projectName === project.name) setBusy(false);
    }
  }, [applyDetailedSnapshot, busy, onRuntimeMutation, project, refresh, selectedTaskId, visibleSnapshot]);

  const changeMode = useCallback(async (mode: TaskControlMode, reason = "") => {
    if (!project?.name || !visibleSnapshot || busy || mode === visibleSnapshot.control_mode) return;
    setBusy(true);
    setError("");
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>("set_task_control_mode_runtime", {
        projectName: project.name,
        mode,
        expectedRevision: visibleSnapshot.project_revision,
        confirmed: visibleSnapshot.control_mode === "SerialTakeover" && mode !== "SerialTakeover",
        reason,
        source: "task_inspector",
      }, 15_000);
      if (scope.current.projectName !== project.name) return;
      if (result.task_control_snapshot) {
        const resultCursor: TaskControlRuntimeCursor = {
          processStartId: result.runtime_snapshot.process_start_id,
          eventSequence: result.runtime_snapshot.task_control_event_sequence,
          projectRevision: result.runtime_snapshot.project.workflow_state.data_revision,
          treeRevision: result.runtime_snapshot.task_control_tree_revision,
          controlActionId: result.runtime_snapshot.task_control_action_id,
          controlActionKnown: true,
          snapshotVersion: result.runtime_snapshot.task_control_snapshot_version,
        };
        if (isTaskControlSnapshotCurrent(result.task_control_snapshot, resultCursor)) {
          applyDetailedSnapshot(result.task_control_snapshot, project);
        }
      }
      onRuntimeMutation?.(result);
      if (!result.task_control_snapshot || !result.task_control.available) await refresh();
    } catch (reason) {
      if (scope.current.projectName === project.name) setError(String(reason));
    } finally {
      if (scope.current.projectName === project.name) setBusy(false);
    }
  }, [applyDetailedSnapshot, busy, onRuntimeMutation, project, refresh, visibleSnapshot]);

  return {
    snapshot: visibleSnapshot,
    selectedTaskId,
    selectionMode: selection.mode,
    selectedNode,
    busy,
    error,
    sourceEventSequence,
    detailsSyncing: Boolean(project?.name) && !snapshotIsCurrent,
    detailFallbackActive: fallbackDecision.active,
    detailFallbackReason: fallbackDecision.reason,
    refresh,
    selectTask,
    followCurrentTask,
    executeAction,
    changeMode,
  };
}
