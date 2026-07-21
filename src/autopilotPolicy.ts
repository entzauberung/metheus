import type {
  AutopilotRecoveryAction,
  AutopilotRunStatus,
  PipelineState,
} from "./types";

export function executionPollingOwnsNextAdvance(state: PipelineState): boolean {
  return state.status === "Running";
}

export interface AutopilotErrorActions {
  canResume: boolean;
  canRetryAdvance: boolean;
  canRegeneratePlan: boolean;
  canPrepareWorkspace: boolean;
  canRefreshWorkspace: boolean;
  canClose: boolean;
}

export function getAutopilotErrorActions(
  runStatus: AutopilotRunStatus,
  recoveryAction: AutopilotRecoveryAction,
): AutopilotErrorActions {
  const isStopped = runStatus === "Paused" || runStatus === "ErrorStopped";
  return {
    canResume:
      isStopped
      && recoveryAction !== "RestoreExecutionBaseline"
      && recoveryAction !== "WaitHumanDecision"
      && recoveryAction !== "RetryAutopilotAdvance"
      && recoveryAction !== "SyncAndClose"
      && recoveryAction !== "RegenerateExecutionPlan"
      && recoveryAction !== "PrepareExecutionWorkspace"
      && recoveryAction !== "ResolveWorkspaceChanges"
      && recoveryAction !== "RunAutomaticRecovery",
    canRetryAdvance:
      runStatus === "ErrorStopped" && recoveryAction === "RetryAutopilotAdvance",
    canRegeneratePlan:
      runStatus === "ErrorStopped" && recoveryAction === "RegenerateExecutionPlan",
    canPrepareWorkspace:
      runStatus === "ErrorStopped" && recoveryAction === "PrepareExecutionWorkspace",
    canRefreshWorkspace:
      runStatus === "ErrorStopped" && recoveryAction === "ResolveWorkspaceChanges",
    canClose: isStopped,
  };
}
