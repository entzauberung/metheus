import type { ProjectSyncState } from "../hooks/useProjectStateSync";

export function createProjectSyncState(
  overrides: Partial<ProjectSyncState> = {},
): ProjectSyncState {
  return {
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
    ...overrides,
  };
}
