/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, RecoveryPresentation } from "../types";
import { RecoveryDecisionDialog } from "./RecoveryDecisionDialog";

function project(): Project {
  return {
    name: "decision-dialog",
    execution_session: { subtask_id: "task-1" },
    workflow_state: {
      recovery_state: { subtask_id: "task-1" },
    },
    milestones: [{
      subtasks: [],
      mid_stages: [{
        subtasks: [{
          id: "task-1",
          child_tasks: [],
          acceptance_criteria: ["保留兼容行为", "记录人工证据"],
          acceptance_ledger: [
            { criterion_index: 1, status: "Satisfied" },
            { criterion_index: 2, status: "Unsatisfied" },
          ],
        }],
      }],
    }],
  } as unknown as Project;
}

function presentation(): RecoveryPresentation {
  return {
    kind: "HumanDecision",
    title: "等待人工决策",
    reason: "验收证据存在冲突",
    severity: "Error",
    primary_action: null,
    secondary_actions: [],
    preserve_current_code: true,
    requires_baseline_restore: false,
    supports_preview: false,
    automatic_retry: false,
    capabilities: ["ResolveHumanRecovery"],
    decision_options: [{
      resolution: "accept_deviation",
      label: "接受偏差并继续",
      enabled: true,
      disabled_reason: null,
      requires_reason: true,
      requires_acceptance_selection: true,
      requires_baseline_preview: false,
    }],
    state_fingerprint: "human-decision-1",
  };
}

describe("RecoveryDecisionDialog", () => {
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

  it("requires the backend-declared reason and acceptance selection", async () => {
    const onSubmit = vi.fn(async () => undefined);
    act(() => root.render(
      <RecoveryDecisionDialog
        isOpen
        project={project()}
        presentation={presentation()}
        busy={false}
        onClose={vi.fn()}
        onSubmit={onSubmit}
      />,
    ));

    const confirm = [...document.body.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("确认处理"));
    expect(confirm?.disabled).toBe(true);

    const textarea = document.body.querySelector<HTMLTextAreaElement>("textarea");
    const criterion = [...document.body.querySelectorAll<HTMLInputElement>("input[type='checkbox']")]
      .find(input => input.parentElement?.textContent?.includes("记录人工证据"));
    act(() => {
      if (textarea) {
        const valueSetter = Object.getOwnPropertyDescriptor(
          HTMLTextAreaElement.prototype,
          "value",
        )?.set;
        valueSetter?.call(textarea, "风险可控，后续任务将补齐证据");
        textarea.dispatchEvent(new Event("input", { bubbles: true }));
      }
      criterion?.click();
    });
    expect(confirm?.disabled).toBe(false);

    await act(async () => {
      confirm?.click();
    });
    expect(onSubmit).toHaveBeenCalledWith({
      resolution: "accept_deviation",
      reason: "风险可控，后续任务将补齐证据",
      acceptedCriteria: [2],
    });
  });
});
