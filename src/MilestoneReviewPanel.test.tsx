/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MilestoneReviewPanel } from "./MilestoneReviewPanel";
import type { Milestone, MilestoneHumanReviewItem } from "./types";

function reviewItem(id: string): MilestoneHumanReviewItem {
  return {
    id,
    milestone_id: "milestone-1",
    task_id: `task-${id}`,
    criterion_index: 1,
    criterion: `验收 ${id}`,
    contract_fingerprint: "contract-1",
    execution_facts_fingerprint: "facts-1",
    review_cycle: 3,
    ai_status: "DeferredHumanReview",
    ai_evidence: "等待人工确认",
    visual_status: "Unavailable",
    visual_summary: "",
    visual_evidence: [],
    human_decision: "Pending",
    human_reason: "",
    updated_at: "2026-08-08T00:00:00Z",
  };
}

function milestone(items: MilestoneHumanReviewItem[] = []): Milestone {
  return {
    id: "milestone-1",
    title: "集中验收",
    human_review_cycle: 3,
    human_review_fingerprint: "review-fingerprint-3",
    human_review_items: items,
  } as Milestone;
}

function button(host: HTMLElement, label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")]
    .find((candidate) => candidate.textContent?.includes(label));
  if (!found) throw new Error(`找不到按钮：${label}`);
  return found;
}

function chooseRadio(host: HTMLElement, index: number) {
  const radios = host.querySelectorAll<HTMLInputElement>('input[type="radio"]');
  act(() => radios[index].click());
}

function setText(element: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    "value",
  )?.set;
  setter?.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("MilestoneReviewPanel", () => {
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

  it("submits the complete current-cycle checklist in one backend payload", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    act(() => root.render(
      <MilestoneReviewPanel
        milestone={milestone([reviewItem("one"), reviewItem("two")])}
        onSubmit={onSubmit}
        projectRevision={17}
      />,
    ));

    act(() => button(host, "A：正常继续").click());
    await act(async () => button(host, "确认继续").click());
    chooseRadio(host, 0);
    chooseRadio(host, 2);
    await act(async () => button(host, "提交人工结论与分支").click());

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith({
      milestone_id: "milestone-1",
      review_cycle: 3,
      expected_revision: 17,
      review_fingerprint: "review-fingerprint-3",
      branch: "A",
      branch_reason: "",
      decisions: [
        { item_id: "one", decision: "Confirmed", reason: "" },
        { item_id: "two", decision: "Confirmed", reason: "" },
      ],
    });
    expect(host.querySelector('[role="dialog"]')).toBeNull();
  });

  it("guards a submission from repeated clicks while the backend is pending", async () => {
    let resolveSubmit: (() => void) | undefined;
    const onSubmit = vi.fn(() => new Promise<void>((resolve) => {
      resolveSubmit = resolve;
    }));
    act(() => root.render(
      <MilestoneReviewPanel
        milestone={milestone()}
        onSubmit={onSubmit}
        projectRevision={17}
      />,
    ));

    act(() => button(host, "A：正常继续").click());
    const confirm = button(host, "确认继续");
    await act(async () => {
      confirm.click();
      confirm.click();
    });
    expect(onSubmit).toHaveBeenCalledTimes(1);

    await act(async () => resolveSubmit?.());
  });

  it("keeps the dialog open and exposes a backend conflict without optimistic state", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("项目 revision 冲突"));
    act(() => root.render(
      <MilestoneReviewPanel
        milestone={milestone([reviewItem("one")])}
        onSubmit={onSubmit}
        projectRevision={17}
      />,
    ));

    act(() => button(host, "B：修正过去").click());
    await act(async () => button(host, "开始讨论修正").click());
    chooseRadio(host, 1);
    const branchReason = host.querySelector<HTMLTextAreaElement>(
      ".milestone-human-review-dialog > .milestone-human-review-reason textarea",
    );
    if (!branchReason) throw new Error("缺少 B 分支修正理由输入框");
    act(() => setText(branchReason, "需要修正验收问题"));
    await act(async () => button(host, "提交人工结论与分支").click());

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("revision 冲突");
  });
});
