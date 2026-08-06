/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Project,
  RecoveryPresentation,
  TaskControlSnapshot,
  TaskTreeNodeView,
} from "./types";
import TaskInspector from "./TaskInspector";

const TASK_CAPABILITIES = ["revalidate", "accept_deviation"];

function node(id: string, capabilities = TASK_CAPABILITIES): TaskTreeNodeView {
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
    capabilities,
    disabled_reasons: capabilities.length > 0 ? {} : {
      revalidate: "非当前任务节点只读",
      accept_deviation: "非当前任务节点只读",
    },
    is_currently_actionable: capabilities.length > 0,
    actionable_acceptance_criteria: capabilities.includes("accept_deviation") ? [1] : [],
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
    source_process_start_id: "process-1",
    source_event_sequence: 4,
    control_mode: "Shadow",
    control_capabilities: ["pause", "stop", "revalidate", "accept_deviation"],
    nodes: [
      node("selected", currentTaskId === "selected" ? TASK_CAPABILITIES : []),
      node(currentTaskId),
    ],
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

function staleControlLockPresentation(): RecoveryPresentation {
  return {
    presentation_version: "1",
    kind: "ControlActionOccupied",
    title: "控制动作占用已失效",
    reason: "原进程已退出",
    severity: "Warning",
    primary_action: {
      capability: "ClearStaleControlLock",
      label: "清理陈旧锁并恢复操作",
      enabled: true,
      disabled_reason: null,
    },
    secondary_actions: [],
    preserve_current_code: true,
    requires_baseline_restore: false,
    supports_preview: false,
    automatic_retry: false,
    capabilities: ["ClearStaleControlLock"],
    decision_options: [],
    state_fingerprint: "stale-lock",
    control_lock_valid: false,
    control_action_description: "执行任务 selected",
    control_action_elapsed_seconds: 37,
    control_lock_last_heartbeat_at: "2026-07-29T00:00:00Z",
    control_lock_failure_reason: "持有进程与当前进程不一致",
    control_lock_cleanup_available: true,
  };
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

  function render(
    currentTaskId = "selected",
    syncState: { error?: string; detailsSyncing?: boolean } = {},
    recoveryPresentation: RecoveryPresentation | null = null,
  ) {
    const onClose = vi.fn();
    const onAction = vi.fn();
    act(() => {
      root.render(
        <TaskInspector
          project={project()}
          snapshot={snapshot(currentTaskId)}
          selectedNode={node(
            "selected",
            currentTaskId === "selected" ? TASK_CAPABILITIES : [],
          )}
          selectedTaskId="selected"
          busy={false}
          error={syncState.error ?? ""}
          recoveryPresentation={recoveryPresentation}
          expectedEventSequence={4}
          detailsSyncing={syncState.detailsSyncing ?? false}
          onClose={onClose}
          onRefresh={vi.fn()}
          onAction={onAction}
          onChangeMode={vi.fn()}
        />,
      );
    });
    return { onClose, onAction };
  }

  it("offers four focused pages and keeps a non-current task read-only", () => {
    render("other-task");
    expect([...host.querySelectorAll('[role="tab"]')].map(tab => tab.getAttribute("title")))
      .toEqual(["概览与合同", "验收与证据", "决策与恢复", "成本与事件"]);
    expect(host.textContent).toContain("非当前任务节点只读");
    const revalidate = [...host.querySelectorAll("button")]
      .find(button => button.textContent?.includes("重新验证"));
    expect(revalidate?.disabled).toBe(true);
  });

  it("shows the frozen workload and executor budgets from the backend contract", () => {
    const selected = node("selected");
    selected.contract = {
      workload_scale: "Small",
      max_split_depth: 0,
      budget: {
        level: "small",
        estimated_model_calls: 1,
        max_executor_turns: 8,
        max_transport_retries: 1,
        max_doom_loop_retries: 0,
      },
      allowed_file_paths: ["index.html"],
      acceptance_criteria: ["页面可用"],
      stop_rules: ["越界时停止"],
      fingerprint: "sha256:contract",
      title: "静态页面",
      goal: "交付页面",
      complexity: "Small",
      risk: "Low",
      depth: 0,
    } as unknown as NonNullable<TaskTreeNodeView["contract"]>;
    act(() => {
      root.render(
        <TaskInspector
          project={project()}
          snapshot={snapshot("selected")}
          selectedNode={selected}
          selectedTaskId="selected"
          busy={false}
          error=""
          recoveryPresentation={null}
          expectedEventSequence={4}
          detailsSyncing={false}
          onClose={vi.fn()}
          onRefresh={vi.fn()}
          onAction={vi.fn()}
          onChangeMode={vi.fn()}
        />,
      );
    });
    expect(host.textContent).toContain("工作负载 Small · 最大拆分深度 0");
    expect(host.textContent).toContain("最多 8 执行轮");
    expect(host.textContent).toContain("transport 1 · Doom Loop 0");
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
    expect(host.textContent).toContain("接受验收偏差");
  });

  it("provability closeout shows a distinct human-review badge and confirmation entry", () => {
    const humanProject = project();
    humanProject.workflow_state.recovery_state = {
      phase: "WaitingHuman",
      subtask_id: "selected",
    } as Project["workflow_state"]["recovery_state"];
    const subtask = humanProject.milestones[0].mid_stages[0].subtasks[0];
    subtask.acceptance_criteria_meta = [{
      text: "恢复并渲染保存的数据",
      provability: "HumanReview",
      provability_source: "PlanningExplicit",
    }];
    const humanNode = node("selected", [...TASK_CAPABILITIES, "confirm_actual_pass"]);
    humanNode.actionable_acceptance_criteria = [1];
    const onConfirmHumanReview = vi.fn();
    act(() => {
      root.render(
        <TaskInspector
          project={humanProject}
          snapshot={snapshot("selected")}
          selectedNode={humanNode}
          selectedTaskId="selected"
          busy={false}
          error=""
          recoveryPresentation={null}
          expectedEventSequence={4}
          detailsSyncing={false}
          onClose={vi.fn()}
          onRefresh={vi.fn()}
          onAction={vi.fn()}
          onConfirmHumanReview={onConfirmHumanReview}
          onChangeMode={vi.fn()}
        />,
      );
    });
    const acceptanceTab = host.querySelector('[role="tab"][title="验收与证据"]');
    act(() => acceptanceTab?.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      button: 0,
    })));

    expect(host.textContent).toContain("人工确认边界");
    expect(host.textContent).toContain("人工确认");
    expect(host.textContent).toContain("我已确认该项");
    expect(host.textContent).not.toContain("AI 证据不足");
  });

  it("does not render deviation controls for a future task", () => {
    render("other-task");
    const acceptanceTab = host.querySelector('[role="tab"][title="验收与证据"]');
    act(() => acceptanceTab?.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      button: 0,
    })));
    expect(host.textContent).not.toContain("接受验收偏差");
    expect(host.querySelector('[data-testid="accept-deviation-disabled-reason"]')?.textContent)
      .toContain("非当前任务节点只读");
  });

  it("removes a stale terminal action when backend node capabilities change", () => {
    render("selected");
    const acceptanceTab = host.querySelector('[role="tab"][title="验收与证据"]');
    act(() => acceptanceTab?.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      button: 0,
    })));
    expect(host.textContent).toContain("接受验收偏差");

    render("other-task");
    expect(host.textContent).not.toContain("接受验收偏差");
    expect(host.querySelector('[data-testid="accept-deviation-disabled-reason"]')?.textContent)
      .toContain("非当前任务节点只读");
  });

  it("closes from the header and Escape without changing the selected task", () => {
    const { onClose } = render();
    act(() => host.querySelector('button[aria-label="关闭任务检查器"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("任务 selected");
  });

  it("keeps detailed snapshot failure non-blocking and visibly retrying", () => {
    render("selected", {
      error: "详细快照生成失败",
      detailsSyncing: true,
    });
    expect(host.textContent).toContain("详细快照生成失败");
    expect(host.textContent).toContain("主状态已更新，正在后台重试");
  });

  it("shows stale control-lock facts without offering a recovery decision", () => {
    render("selected", {}, staleControlLockPresentation());
    const recoveryTab = host.querySelector('[role="tab"][title="决策与恢复"]');
    act(() => recoveryTab?.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      button: 0,
    })));

    const details = host.querySelector(".task-recovery-details");
    expect(details?.textContent).toContain("执行任务 selected");
    expect(details?.textContent).toContain("37 秒");
    expect(details?.textContent).toContain("持有进程与当前进程不一致");
    expect(host.textContent).not.toContain("选择人工恢复处理方式");
  });
});
