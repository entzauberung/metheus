import type { ExecutionWorkspaceStatus } from "./types";

export type WorkspaceAction =
  | "none"
  | "prepare"
  | "configure_identity"
  | "resolve_changes"
  | "refresh";

export function getWorkspaceAction(
  status: ExecutionWorkspaceStatus | null,
): WorkspaceAction {
  if (!status) return "refresh";
  if (status.ready) return "none";
  if (
    status.issues.includes("MissingGitUserName")
    || status.issues.includes("MissingGitUserEmail")
  ) {
    return "configure_identity";
  }
  if (status.has_commits && status.issues.includes("DirtyWorkingTree")) {
    return "resolve_changes";
  }
  if (
    status.issues.includes("NotGitRepository")
    || status.issues.includes("NoCommits")
  ) {
    return "prepare";
  }
  if (status.issues.includes("DirtyWorkingTree")) return "resolve_changes";
  return "refresh";
}
