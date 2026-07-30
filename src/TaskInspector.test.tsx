/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, TaskControlSnapshot, TaskTreeNodeView } from "./types";
import TaskInspector from "./TaskInspector";

function node(id: string): TaskTreeNodeView {
  return {
    id,
    title: `任务 ${id}`,
    node_type: "Subtask",
    status: "AwaitingConfirmation",
    depth: 3,
    complexity: "Medium",
    risk: "High",
    contract_fingerprint: "contract-a",
    dependencies: [],
    acceptance: [{
      criterion_index: 1,
      criterion: "恢复并渲染保存的数据",
      status: "Unknown",
      evidence: "需要读取入口",
      evidence_references: [{
        block_id: "E004",
        source_kind: "CurrentFileSnippet",
        file: "index.html",
        start_line: 210,
        end_line: 238,
      }],
      confidence: 0.6,
      updated_at: "2026-07-29T00:00:00Z",
    }],
    children: [],
  };
}

function project(): Project {
  return {
    name: "inspector-test",
    workflow_state: { data_revision: 1 },
    milestones: [{
      id: "milestone",
      subtasks: [],
      mid_stages: [{
        id: "mid",
        subtasks: [{
          id: "selected",
          title: "任务 selected",
          status: "AwaitingConfirmation",
          child_tasks: [],
          depends_on: [],
          acceptance_ledger: node("selected").acceptance,
          test_result: {
            passed: false,
            issues: [],
            suggestion: "",
            automated_test_status: "NotConfigured",
            verification_kind: "CodeReviewOnly",
          },
        }],
      }],
    }],
  } as unknown as Project;
}

function snapshot(currentTaskId: string): TaskControlSnapshot {
  return {
    project_name: "inspector-test",
    project_revision: 1,
    current_task_id: currentTaskId,
    task_tree_revision: 1,
    control_mode: "Shadow",
    control_capabilities: ["pause", "stop", "revalidate", "accept_deviation"],
    nodes: [node("selected"), node(currentTaskId)],
    events: [{
      timestamp: "2026-07-29T00:00:00Z",
      level: "info",
      source: "Controller",
      text: "等待补证",
      task_id: "selected",
      criterion_index: 1,
      validator_id: "semantic_review",
    }],
  } as unknown as TaskControlSnapshot;
}

describe("TaskInspector", () => {
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
    vi.restoreAllMocks();
  });

  function render(currentTaskId = "selected") {
    const onClose = vi.fn();
    const onAction = vi.fn();
    act(() => {
      root.render(
        <TaskInspector
          project={project()}
          snapshot={snapshot(currentTaskId)}
          selectedNode={node("selected")}
          selectedTaskId="selected"
          busy={false}
          error=""
          onClose={onClose}
          onRefresh={vi.fn()}
          onAction={onAction}
          onChangeMode={vi.fn()}
        />,
      );
    });
    return { onClose, onAction };
  }

  it("offers four focused pages and disables task actions for history", () => {
    render("other-task");
    expect([...host.querySelectorAll('[role="tab"]')].map(tab => tab.getAttribute("title")))
      .toEqual(["概览与合同", "验收与证据", "决策与恢复", "成本与事件"]);
    expect(host.textContent).toContain("当前正在查看历史任务");
    const revalidate = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("重新验证"));
    expect(revalidate?.disabled).toBe(true);
  });

  it("shows test status and exact evidence lines on the acceptance page", () => {
    render();
    const acceptanceTab = host.querySelector('[role="tab"][title="验收与证据"]');
    act(() => acceptanceTab?.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      button: 0,
    })));

    expect(host.textContent).toContain("NotConfigured");
    expect(host.textContent).toContain("CodeReviewOnly");
    expect(host.textContent).toContain("E004");
    expect(host.textContent).toContain("index.html:210-238");
  });

  it("closes from the header and Escape without changing the selected task", () => {
    const { onClose } = render();
    act(() => host.querySelector('button[aria-label="关闭任务检查器"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("任务 selected");
  });
});
