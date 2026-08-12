import { useCallback, useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import type {
  ProjectStateChangedEvent,
  ProjectStateSubscription,
  RuntimeSnapshot,
  TaskControlDetailStatus,
  TaskControlMode,
} from "../types";
import {
  advanceProjectSyncCursor,
  advanceProjectSyncRevisions,
  mergePendingProjectEvent,
  PROJECT_SYNC_CONNECTED_FALLBACK_MS,
  projectSyncFallbackIntervalMs,
  recoveryPresentationIsActive,
  shouldAcceptProjectStateEvent,
  shouldAcceptRuntimeSnapshot,
  shouldRequestRuntimeSnapshot,
  type ProjectSyncCursor,
} from "../projectSyncPolicy";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";

const DEFAULT_COALESCE_MS = 40;
const DEFAULT_FALLBACK_INTERVAL_MS = PROJECT_SYNC_CONNECTED_FALLBACK_MS;
const DISCONNECTED_FAILURE_COUNT = 3;

interface ProjectEventChannel {
  onmessage: (event: ProjectStateChangedEvent) => void;
}

export interface ProjectStateSyncTransport {
  createChannel: (onmessage: (event: ProjectStateChangedEvent) => void) => ProjectEventChannel;
  subscribe: (
    projectName: string,
    channel: ProjectEventChannel,
  ) => Promise<ProjectStateSubscription>;
  unsubscribe: (subscriptionId: string) => Promise<void>;
  getSnapshot: (
    projectName: string,
    includeTaskControlSnapshot?: boolean,
  ) => Promise<RuntimeSnapshot>;
}

const defaultTransport: ProjectStateSyncTransport = {
  createChannel(onmessage) {
    const channel = new Channel<ProjectStateChangedEvent>();
    channel.onmessage = onmessage;
    return channel;
  },
  subscribe(projectName, channel) {
    return invokeWithTimeout<ProjectStateSubscription>("subscribe_project_state", {
      projectName,
      onEvent: channel,
    }, 10_000);
  },
  unsubscribe(subscriptionId) {
    return invokeWithTimeout<void>("unsubscribe_project_state", { subscriptionId }, 10_000);
  },
  getSnapshot(projectName, includeTaskControlSnapshot = false) {
    return invokeWithTimeout<RuntimeSnapshot>("get_runtime_snapshot", {
      projectName,
      includeTaskControlSnapshot,
    }, 10_000);
  },
};

export type ProjectSyncStatus = "idle" | "syncing" | "synced" | "delayed" | "disconnected";

export interface ProjectSyncState {
  status: ProjectSyncStatus;
  subscriptionStatus: "idle" | "connected" | "reconnecting";
  lastSuccessfulSyncAt: string | null;
  consecutiveFailures: number;
  lastEventSequence: number;
  pendingRevision: number | null;
  lastError: string;
  taskControlEventSequence: number;
  taskControlProcessStartId: string;
  taskControlProjectRevision: number;
  taskControlTreeRevision: number;
  taskControlDirty: boolean;
  taskControlSnapshotVersion: string;
  taskControlActionId: string | null;
  taskControlMode: TaskControlMode | null;
  taskControlDetailStatus: TaskControlDetailStatus;
  taskControlDetailUpdatedAt: string | null;
}

interface UseProjectStateSyncOptions {
  projectName: string;
  enabled?: boolean;
  onSnapshot: (snapshot: RuntimeSnapshot) => void;
  coalesceMs?: number;
  fallbackIntervalMs?: number;
  includeTaskControlSnapshot?: boolean;
  transport?: ProjectStateSyncTransport;
}

export interface ProjectStateSyncController {
  state: ProjectSyncState;
  forceSync: () => Promise<RuntimeSnapshot | null>;
}

const initialState: ProjectSyncState = {
  status: "idle",
  subscriptionStatus: "idle",
  lastSuccessfulSyncAt: null,
  consecutiveFailures: 0,
  lastEventSequence: 0,
  pendingRevision: null,
  lastError: "",
  taskControlEventSequence: 0,
  taskControlProcessStartId: "",
  taskControlProjectRevision: 0,
  taskControlTreeRevision: 0,
  taskControlDirty: false,
  taskControlSnapshotVersion: "",
  taskControlActionId: null,
  taskControlMode: null,
  taskControlDetailStatus: "idle",
  taskControlDetailUpdatedAt: null,
};

export function useProjectStateSync({
  projectName,
  enabled = true,
  onSnapshot,
  coalesceMs = DEFAULT_COALESCE_MS,
  fallbackIntervalMs = DEFAULT_FALLBACK_INTERVAL_MS,
  includeTaskControlSnapshot = false,
  transport = defaultTransport,
}: UseProjectStateSyncOptions): ProjectStateSyncController {
  const [state, setState] = useState<ProjectSyncState>(initialState);
  const [activeRecovery, setActiveRecovery] = useState(false);
  const scopeRef = useRef({ generation: 0, projectName: "", enabled: false });
  const cursorRef = useRef<ProjectSyncCursor>({
    projectName: "",
    processStartId: "",
    eventSequence: 0,
    dataRevision: 0,
    taskControlTreeRevision: 0,
    taskControlSnapshotVersion: "",
    taskControlActionId: null,
    taskControlMode: null,
  });
  const inFlightRef = useRef<Promise<RuntimeSnapshot | null> | null>(null);
  const inFlightGenerationRef = useRef(0);
  const requestedSyncRef = useRef(0);
  const completedSyncRef = useRef(0);
  const coalesceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingEventRef = useRef<ProjectStateChangedEvent | null>(null);
  const channelRef = useRef<ProjectEventChannel | null>(null);
  const subscriptionIdRef = useRef("");
  const subscriptionRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onSnapshotRef = useRef(onSnapshot);
  const transportRef = useRef(transport);
  const includeTaskControlSnapshotRef = useRef(includeTaskControlSnapshot);
  const previousIncludeTaskControlSnapshotRef = useRef(includeTaskControlSnapshot);
  const syncNowRef = useRef<(force?: boolean) => Promise<RuntimeSnapshot | null>>(async () => null);
  onSnapshotRef.current = onSnapshot;
  transportRef.current = transport;
  includeTaskControlSnapshotRef.current = includeTaskControlSnapshot;

  const syncNow = useCallback(async (force = false) => {
    const scope = scopeRef.current;
    if (!scope.enabled || !scope.projectName) return null;
    requestedSyncRef.current += 1;
    if (inFlightRef.current && inFlightGenerationRef.current === scope.generation) {
      return await inFlightRef.current;
    }
    const requestScope = { ...scope };
    if (force && coalesceTimerRef.current) {
      clearTimeout(coalesceTimerRef.current);
      coalesceTimerRef.current = null;
    }
    setState(current => ({
      ...current,
      status: "syncing",
      taskControlDetailStatus: includeTaskControlSnapshotRef.current ? "syncing" : "idle",
    }));

    let task!: Promise<RuntimeSnapshot | null>;
    task = (async () => {
      let latest: RuntimeSnapshot | null = null;
      try {
        while (
          scopeRef.current.generation === requestScope.generation
          && completedSyncRef.current < requestedSyncRef.current
        ) {
          const targetRequest = requestedSyncRef.current;
          const includeDetail = includeTaskControlSnapshotRef.current;
          try {
            const snapshot = await transportRef.current.getSnapshot(
              requestScope.projectName,
              includeDetail,
            );
            const currentScope = scopeRef.current;
            if (
              currentScope.generation !== requestScope.generation
              || currentScope.projectName !== requestScope.projectName
            ) return null;
            if (!shouldAcceptRuntimeSnapshot(cursorRef.current, snapshot)) {
              latest = null;
              if (includeDetail) {
                setState(current => ({
                  ...current,
                  taskControlDetailStatus: "unavailable",
                }));
              }
            } else {
              cursorRef.current = advanceProjectSyncCursor(
                cursorRef.current,
                snapshot.process_start_id,
                snapshot.event_sequence,
              );
              cursorRef.current = advanceProjectSyncRevisions(
                cursorRef.current,
                snapshot.project.workflow_state.data_revision,
                snapshot.task_control_tree_revision,
                snapshot.task_control_snapshot_version,
                snapshot.task_control_action_id,
                snapshot.task_control_mode,
              );
              pendingEventRef.current = null;
              setActiveRecovery(recoveryPresentationIsActive(snapshot.recovery_presentation));
              onSnapshotRef.current(snapshot);
              latest = snapshot;
              setState(current => {
                const taskControlChanged = current.taskControlDirty
                  || snapshot.task_control_snapshot_version !== current.taskControlSnapshotVersion
                  || snapshot.task_control_tree_revision !== current.taskControlTreeRevision
                  || snapshot.task_control_action_id !== current.taskControlActionId
                  || snapshot.task_control_mode !== current.taskControlMode;
                return {
                  ...current,
                  status: current.subscriptionStatus === "connected" ? "synced" : "delayed",
                  lastSuccessfulSyncAt: new Date().toISOString(),
                  consecutiveFailures: 0,
                  lastEventSequence: cursorRef.current.eventSequence,
                  pendingRevision: null,
                  lastError: current.subscriptionStatus === "connected"
                    ? ""
                    : "状态通知通道正在重连，当前使用低频快照兜底",
                  taskControlEventSequence: taskControlChanged
                    ? Math.max(current.taskControlEventSequence, snapshot.task_control_event_sequence)
                    : current.taskControlEventSequence,
                  taskControlProcessStartId: snapshot.process_start_id,
                  taskControlProjectRevision: snapshot.project.workflow_state.data_revision,
                  taskControlTreeRevision: snapshot.task_control_tree_revision,
                  taskControlDirty: false,
                  taskControlSnapshotVersion: snapshot.task_control_snapshot_version,
                  taskControlActionId: snapshot.task_control_action_id,
                  taskControlMode: snapshot.task_control_mode,
                  taskControlDetailStatus: includeDetail
                    ? snapshot.task_control_snapshot
                      ? "ready"
                      : "unavailable"
                    : "idle",
                  taskControlDetailUpdatedAt: includeDetail && snapshot.task_control_snapshot
                    ? new Date().toISOString()
                    : current.taskControlDetailUpdatedAt,
                };
              });
            }
          } catch (reason) {
            latest = null;
            if (scopeRef.current.generation !== requestScope.generation) return null;
            setState(current => {
              const failures = current.consecutiveFailures + 1;
              return {
                ...current,
                status: failures >= DISCONNECTED_FAILURE_COUNT ? "disconnected" : "delayed",
                consecutiveFailures: failures,
                lastError: String(reason),
                taskControlDetailStatus: includeDetail ? "unavailable" : "idle",
              };
            });
          }
          completedSyncRef.current = targetRequest;
        }
        return latest;
      } finally {
        if (inFlightRef.current === task) inFlightRef.current = null;
      }
    })();
    inFlightRef.current = task;
    inFlightGenerationRef.current = requestScope.generation;
    return await task;
  }, []);
  syncNowRef.current = syncNow;

  const scheduleEventSync = useCallback((event: ProjectStateChangedEvent) => {
    const cursor = cursorRef.current;
    if (!shouldAcceptProjectStateEvent(cursor, event)) return;
    const requestSnapshot = shouldRequestRuntimeSnapshot(cursor, event);
    const taskControlChanged = requestSnapshot && event.task_control_dirty;
    cursorRef.current = advanceProjectSyncCursor(
      cursor,
      event.process_start_id,
      event.event_sequence,
    );
    cursorRef.current = advanceProjectSyncRevisions(
      cursorRef.current,
      event.data_revision,
      event.task_control_tree_revision,
      event.task_control_snapshot_version,
      event.control_action_id,
      event.control_mode,
    );
    if (requestSnapshot) {
      pendingEventRef.current = mergePendingProjectEvent(pendingEventRef.current, event);
    }
    setState(current => ({
      ...current,
      lastEventSequence: cursorRef.current.eventSequence,
      pendingRevision: requestSnapshot
        ? Math.max(current.pendingRevision ?? 0, event.data_revision)
        : current.pendingRevision,
      taskControlEventSequence: taskControlChanged
        ? event.event_sequence
        : current.taskControlEventSequence,
      taskControlProcessStartId: taskControlChanged
        ? event.process_start_id
        : current.taskControlProcessStartId,
      taskControlProjectRevision: taskControlChanged
        ? event.data_revision
        : current.taskControlProjectRevision,
      taskControlTreeRevision: taskControlChanged
        ? event.task_control_tree_revision
        : current.taskControlTreeRevision,
      taskControlDirty: current.taskControlDirty || taskControlChanged,
      taskControlActionId: taskControlChanged
        ? event.control_action_id
        : current.taskControlActionId,
      taskControlMode: taskControlChanged
        ? event.control_mode
        : current.taskControlMode,
      taskControlDetailStatus: taskControlChanged && includeTaskControlSnapshotRef.current
        ? "syncing"
        : current.taskControlDetailStatus,
    }));
    if (!requestSnapshot) return;
    if (coalesceTimerRef.current) return;
    coalesceTimerRef.current = setTimeout(() => {
      coalesceTimerRef.current = null;
      void syncNowRef.current();
    }, coalesceMs);
  }, [coalesceMs]);

  useEffect(() => {
    scopeRef.current = {
      generation: scopeRef.current.generation + 1,
      projectName,
      enabled: enabled && Boolean(projectName),
    };
    cursorRef.current = {
      projectName,
      processStartId: "",
      eventSequence: 0,
      dataRevision: 0,
      taskControlTreeRevision: 0,
      taskControlSnapshotVersion: "",
      taskControlActionId: null,
      taskControlMode: null,
    };
    pendingEventRef.current = null;
    requestedSyncRef.current = 0;
    completedSyncRef.current = 0;
    subscriptionIdRef.current = "";
    setState(initialState);
    setActiveRecovery(false);
    if (!enabled || !projectName) return;

    let cancelled = false;
    const generation = scopeRef.current.generation;
    let subscribeFailures = 0;
    const subscribeChannel = () => {
      if (cancelled || scopeRef.current.generation !== generation) return;
      const channel = transportRef.current.createChannel(scheduleEventSync);
      channelRef.current = channel;
      setState(current => ({ ...current, subscriptionStatus: "reconnecting" }));
      void transportRef.current.subscribe(projectName, channel)
        .then(subscription => {
          if (cancelled || scopeRef.current.generation !== generation) {
            void transportRef.current.unsubscribe(subscription.subscription_id).catch(() => {});
            return;
          }
          subscribeFailures = 0;
          subscriptionIdRef.current = subscription.subscription_id;
          cursorRef.current = advanceProjectSyncCursor(
            cursorRef.current,
            subscription.process_start_id,
            subscription.event_sequence,
          );
          setState(current => ({
            ...current,
            subscriptionStatus: "connected",
            status: current.lastSuccessfulSyncAt ? "synced" : current.status,
            lastEventSequence: cursorRef.current.eventSequence,
            lastError: current.lastSuccessfulSyncAt ? "" : current.lastError,
          }));
        })
        .catch(reason => {
          if (cancelled || scopeRef.current.generation !== generation) return;
          subscribeFailures += 1;
          setState(current => ({
            ...current,
            status: "delayed",
            subscriptionStatus: "reconnecting",
            lastError: `状态通知订阅失败：${String(reason)}`,
          }));
          const retryMs = Math.min(1_000 * (2 ** Math.min(subscribeFailures - 1, 4)), 15_000);
          subscriptionRetryTimerRef.current = setTimeout(subscribeChannel, retryMs);
        });
    };
    subscribeChannel();

    void syncNowRef.current(true);
    return () => {
      cancelled = true;
      scopeRef.current = {
        generation: scopeRef.current.generation + 1,
        projectName: "",
        enabled: false,
      };
      if (subscriptionRetryTimerRef.current) clearTimeout(subscriptionRetryTimerRef.current);
      subscriptionRetryTimerRef.current = null;
      if (coalesceTimerRef.current) clearTimeout(coalesceTimerRef.current);
      coalesceTimerRef.current = null;
      channelRef.current = null;
      const subscriptionId = subscriptionIdRef.current;
      subscriptionIdRef.current = "";
      if (subscriptionId) {
        void transportRef.current.unsubscribe(subscriptionId).catch(() => {});
      }
    };
  }, [enabled, projectName, scheduleEventSync]);

  useEffect(() => {
    if (!enabled || !projectName) return;
    const intervalMs = projectSyncFallbackIntervalMs(
      state.subscriptionStatus,
      state.status,
      fallbackIntervalMs,
      activeRecovery,
    );
    const fallbackTimer = setInterval(() => {
      void syncNowRef.current();
    }, intervalMs);
    return () => clearInterval(fallbackTimer);
  }, [
    activeRecovery,
    enabled,
    fallbackIntervalMs,
    projectName,
    state.status,
    state.subscriptionStatus,
  ]);

  useEffect(() => {
    const previous = previousIncludeTaskControlSnapshotRef.current;
    previousIncludeTaskControlSnapshotRef.current = includeTaskControlSnapshot;
    if (!includeTaskControlSnapshot) {
      setState(current => ({
        ...current,
        taskControlDetailStatus: "idle",
        taskControlDetailUpdatedAt: null,
      }));
    }
    if (!previous && includeTaskControlSnapshot && enabled && projectName) {
      void syncNowRef.current(true);
    }
  }, [enabled, includeTaskControlSnapshot, projectName]);

  const forceSync = useCallback(async () => {
    const snapshot = await syncNowRef.current(true);
    if (inFlightRef.current) return await inFlightRef.current;
    return snapshot;
  }, []);

  return { state, forceSync };
}
