import type { ExecutionWorkspaceStatus } from "./types";

export type WorkspaceAction =
  | "none"
  | "prepare"
  | "configure_identity"
  | "resolve_changes"
  | "managed_task_changes"
  | "refresh";

export function getWorkspaceAction(
  status: ExecutionWorkspaceStatus | null,
): WorkspaceAction {
  if (!status) return "refresh";
  if (status.ready_for_new_execution) return "none";
  if (
    status.issues.includes("MissingGitUserName")
    || status.issues.includes("MissingGitUserEmail")
  ) {
    return "configure_identity";
  }
  if (
    status.issues.includes("NotGitRepository")
    || status.issues.includes("NoCommits")
  ) {
    return "prepare";
  }
  if (status.has_external_changes) {
    return "resolve_changes";
  }
  if (status.has_managed_task_changes) return "managed_task_changes";
  if (status.issues.includes("DirtyWorkingTree")) return "resolve_changes";
  return "refresh";
}
