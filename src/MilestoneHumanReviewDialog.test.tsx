/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MilestoneHumanReviewDialog } from "./MilestoneHumanReviewDialog";
import type { MilestoneHumanReviewItem } from "./types";

function item(
  id: string,
  humanDecision: MilestoneHumanReviewItem["human_decision"] = "Pending",
): MilestoneHumanReviewItem {
  return {
    id,
    milestone_id: "milestone-1",
    task_id: id === "item-2" ? "task-2" : "task-1",
    criterion_index: 1,
    criterion: `验收 ${id}`,
    contract_fingerprint: "contract-1",
    execution_facts_fingerprint: "facts-1",
    review_cycle: 2,
    ai_status: "DeferredHumanReview",
    ai_evidence: "需要真实人工确认",
    visual_status: "Unavailable",
    visual_summary: "",
    visual_evidence: [],
    human_decision: humanDecision,
    human_reason: humanDecision === "Confirmed" ? "刷新前已确认" : "",
    updated_at: "2026-08-07T00:00:00Z",
  };
}

function button(host: HTMLElement, label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")]
    .find((candidate) => candidate.textContent?.includes(label));
  if (!found) throw new Error(`找不到按钮：${label}`);
  return found;
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

describe("MilestoneHumanReviewDialog", () => {
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

  it.each(["A", "C"] as const)("requires every item to be confirmed for branch %s", async (branch) => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    act(() => root.render(
      <MilestoneHumanReviewDialog
        branch={branch}
        busy={false}
        items={[item("item-1"), item("item-2")]}
        onCancel={vi.fn()}
        onSubmit={onSubmit}
      />,
    ));

    const submit = button(host, "提交人工结论与分支");
    expect(submit.disabled).toBe(true);
    const confirmations = host.querySelectorAll<HTMLInputElement>(
      'input[type="radio"]:not(:checked)',
    );
    act(() => confirmations[0].click());
    expect(submit.disabled).toBe(true);
    act(() => confirmations[2].click());
    expect(submit.disabled).toBe(false);
    await act(async () => submit.click());

    expect(onSubmit).toHaveBeenCalledWith([
      { item_id: "item-1", decision: "Confirmed", reason: "" },
      { item_id: "item-2", decision: "Confirmed", reason: "" },
    ], "");
  });

  it("keeps an existing persisted confirmation and does not require re-entry", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    act(() => root.render(
      <MilestoneHumanReviewDialog
        branch="A"
        busy={false}
        items={[item("item-1", "Confirmed")]}
        onCancel={vi.fn()}
        onSubmit={onSubmit}
      />,
    ));

    const submit = button(host, "提交人工结论与分支");
    expect(host.querySelector<HTMLInputElement>('input[type="radio"]')?.checked).toBe(true);
    expect(submit.disabled).toBe(false);
    await act(async () => submit.click());
    expect(onSubmit).toHaveBeenCalledWith([
      { item_id: "item-1", decision: "Confirmed", reason: "刷新前已确认" },
    ], "");
  });

  it("allows B after every item is handled and records a rejection reason", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    act(() => root.render(
      <MilestoneHumanReviewDialog
        branch="B"
        busy={false}
        items={[item("item-1")]}
        onCancel={vi.fn()}
        onSubmit={onSubmit}
      />,
    ));

    const radios = host.querySelectorAll<HTMLInputElement>('input[type="radio"]');
    act(() => radios[1].click());
    const itemReason = host.querySelector<HTMLTextAreaElement>(
      ".milestone-human-review-item textarea",
    );
    if (!itemReason) throw new Error("缺少逐项说明输入框");
    act(() => setText(itemReason, "390px 视口遮挡"));
    const submit = button(host, "提交人工结论与分支");
    expect(submit.disabled).toBe(true);
    const branchReason = host.querySelector<HTMLTextAreaElement>(
      ".milestone-human-review-dialog > .milestone-human-review-reason textarea",
    );
    if (!branchReason) throw new Error("缺少 B 分支修正理由输入框");
    act(() => setText(branchReason, "需要回到过去修正布局"));
    expect(submit.disabled).toBe(false);
    await act(async () => submit.click());

    expect(onSubmit).toHaveBeenCalledWith([
      { item_id: "item-1", decision: "Rejected", reason: "390px 视口遮挡" },
    ], "需要回到过去修正布局");
  });

  it("rejects empty B even with a branch reason and blocks every control while busy", () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    act(() => root.render(
      <MilestoneHumanReviewDialog
        branch="B"
        busy={false}
        items={[]}
        onCancel={vi.fn()}
        onSubmit={onSubmit}
      />,
    ));
    const submit = button(host, "提交人工结论与分支");
    expect(submit.disabled).toBe(true);
    const branchReason = host.querySelector<HTMLTextAreaElement>(
      ".milestone-human-review-dialog > .milestone-human-review-reason textarea",
    );
    if (!branchReason) throw new Error("缺少 B 分支修正理由输入框");
    act(() => setText(branchReason, "需要回到过去修正"));
    expect(submit.disabled).toBe(true);

    act(() => root.render(
      <MilestoneHumanReviewDialog
        branch="B"
        busy
        items={[]}
        onCancel={vi.fn()}
        onSubmit={onSubmit}
      />,
    ));
    expect(button(host, "原子提交中").disabled).toBe(true);
    expect(button(host, "取消").disabled).toBe(true);
  });
});
