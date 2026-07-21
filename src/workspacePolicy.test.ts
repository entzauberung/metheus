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
});
