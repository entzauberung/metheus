/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MilestoneDraft, Project } from "./types";

vi.mock("./ChatRoom", () => ({
  default: ({ threadId }: { threadId: string }) => (
    <div data-testid="future-chat" data-thread-id={threadId}>专属聊天</div>
  ),
}));

import { FuturePlanningWorkspace } from "./FuturePlanningWorkspace";

function futureDraft(expired: boolean): MilestoneDraft {
  return {
    draft_id: "future-draft",
    status: "Pending",
    draft_kind: "FutureOnly",
    candidate_milestones: [{
      id: "milestone-2",
      title: "未来阶段",
      version: "v0.2",
      description: "future",
      goal: "goal",
      scope: "scope",
      acceptance_criteria: ["accepted"],
    } as MilestoneDraft["candidate_milestones"][number]],
    generation_revision: 2,
    source_plan_revision: 10,
    source_thread_id: "thread-future",
    source_thread_revision: 2,
    source_data_revision: 10,
    expired,
    expiration_reason: expired ? "来源讨论线程新增了消息" : undefined,
    generated_at: "2026-07-30T00:00:00Z",
    regeneration_count: 0,
    split_after_milestone_id: "milestone-1",
    retained_milestone_ids: ["milestone-1"],
    future_candidate_ids: ["milestone-2"],
    original_ai_versions: ["v0.2"],
    normalized_versions: ["v0.2"],
    versions_normalized: true,
    original_remaining_count: 1,
    new_future_count: 1,
    count_expansion_warning: false,
    granularity_check_passed: true,
    granularity_issues: [],
  };
}

function project(expired: boolean, withDraft = true): Project {
  return {
    name: "future-workspace",
    workflow_state: {
      current_step: withDraft ? "FuturePlanApproval" : "BranchDiscussion",
      discussion_scope: "AdjustFuture",
      active_discussion_thread_id: "thread-future",
      data_revision: 11,
    },
    discussion_threads: [{
      id: "thread-future",
      title: "调整未来",
      node_id: "milestone-1",
      messages: [],
      scope: "AdjustFuture",
      milestone_id: "milestone-1",
      review_cycle_id: "cycle-1",
      revision: 2,
      opened_at: "2026-07-30T00:00:00Z",
      status: "Open",
    }],
    milestones: [{
      id: "milestone-1",
      title: "已完成阶段",
      version: "v0.1",
      description: "done",
      status: "Completed",
    }],
    milestone_draft: withDraft ? futureDraft(expired) : undefined,
  } as unknown as Project;
}

describe("FuturePlanningWorkspace", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it("keeps chat visible and disables approval for an expired draft", () => {
    act(() => {
      root.render(
        <FuturePlanningWorkspace
          project={project(true)}
          busy={false}
          onGenerate={vi.fn()}
          onApprove={vi.fn()}
          onProjectUpdated={vi.fn()}
          onAddMessage={vi.fn()}
        />,
      );
    });

    expect(host.querySelector("[data-testid='future-chat']")).not.toBeNull();
    expect(host.textContent).toContain("草稿已过期");
    const approve = Array.from(host.querySelectorAll("button"))
      .find(button => button.textContent?.includes("批准未来规划"));
    expect(approve?.disabled).toBe(true);
  });

  it("uses the same chat workspace before a draft exists", () => {
    act(() => {
      root.render(
        <FuturePlanningWorkspace
          project={project(false, false)}
          busy={false}
          onGenerate={vi.fn()}
          onApprove={vi.fn()}
          onProjectUpdated={vi.fn()}
          onAddMessage={vi.fn()}
        />,
      );
    });

    expect(host.querySelector("[data-testid='future-chat']")).not.toBeNull();
    expect(host.textContent).toContain("尚未生成草稿");
    expect(host.textContent).not.toContain("批准未来规划");
  });
});
