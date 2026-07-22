import { describe, expect, it } from "vitest";
import type { ExecutionWorkspaceIssue, ExecutionWorkspaceStatus } from "./types";
import { getWorkspaceAction } from "./workspacePolicy";

function workspace(issues: ExecutionWorkspaceIssue[]): ExecutionWorkspaceStatus {
  return {
    path_exists: true,
    is_directory: true,
    is_git_repo: !issues.includes("NotGitRepository"),
    has_commits: !issues.includes("NoCommits"),
    git_user_available: !issues.includes("MissingGitUserName"),
    git_email_available: !issues.includes("MissingGitUserEmail"),
    working_tree_clean: !issues.includes("DirtyWorkingTree"),
    git_metadata_ready: !issues.some(issue => [
      "NotGitRepository", "NoCommits", "MissingGitUserName", "MissingGitUserEmail",
    ].includes(issue)),
    ready_for_new_execution: issues.length === 0,
    has_managed_task_changes: false,
    has_external_changes: issues.includes("DirtyWorkingTree"),
    ready: issues.length === 0,
    status_message: "",
    issues,
    changes: [],
  };
}

describe("execution workspace action policy", () => {
  it("only prepares repositories that are missing Git metadata", () => {
    expect(getWorkspaceAction(workspace(["NotGitRepository"]))).toBe("prepare");
    expect(getWorkspaceAction(workspace(["NoCommits"]))).toBe("prepare");
  });

  it("never routes dirty workspaces back to preparation", () => {
    expect(getWorkspaceAction(workspace(["DirtyWorkingTree"]))).toBe("resolve_changes");
  });

  it("allows initial commit preparation when a repository has no HEAD yet", () => {
    expect(
      getWorkspaceAction(workspace(["NoCommits", "DirtyWorkingTree"])),
    ).toBe("prepare");
  });

  it("separates missing identity and ready states", () => {
    expect(getWorkspaceAction(workspace(["MissingGitUserEmail"]))).toBe(
      "configure_identity",
    );
    expect(getWorkspaceAction(workspace([]))).toBe("none");
  });

  it("does not ask users to clean managed task changes", () => {
    const status = workspace(["DirtyWorkingTree"]);
    status.has_external_changes = false;
    status.has_managed_task_changes = true;
    expect(getWorkspaceAction(status)).toBe("managed_task_changes");
  });
});
