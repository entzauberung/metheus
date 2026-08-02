/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Project,
  RecoveryActionPresentation,
  RecoveryCapability,
  RecoveryPresentation,
  RecoveryPresentationKind,
} from "../types";
import { AutopilotControlBar } from "./AutopilotControlBar";

function project(): Project {
  return {
    name: "recovery-ui",
    milestones: [],
    workflow_state: {
      top_level_phase: "Console",
      current_step: "Execution",
      autopilot_active: true,
      autopilot_target_milestone_id: "milestone-1",
      autopilot_state: {
        active: true,
        target_milestone_id: "milestone-1",
        run_status: "Paused",
        last_action: "等待处理",
        last_action_at: "2026-07-31T00:00:00Z",
        error_message: "",
        recovery_action: "None",
      },
    },
  } as unknown as Project;
}

function action(capability: RecoveryCapability, label: string): RecoveryActionPresentation {
  return { capability, label, enabled: true, disabled_reason: null };
}

function presentation(
  kind: RecoveryPresentationKind,
  primaryAction: RecoveryActionPresentation | null,
  options: Partial<RecoveryPresentation> = {},
): RecoveryPresentation {
  const sync = action("SyncProject", "同步状态");
  return {
    kind,
    title: kind === "None" ? "" : `${kind} 标题`,
    reason: kind === "None" ? "" : `${kind} 原因`,
    severity: kind === "None" ? "Info" : "Error",
    primary_action: primaryAction,
    secondary_actions: kind === "None" ? [] : [sync],
    preserve_current_code: kind !== "BaselineRecovery",
    requires_baseline_restore: kind === "BaselineRecovery",
    supports_preview: kind === "BaselineRecovery",
    automatic_retry: false,
    capabilities: [
      ...(kind === "None" ? [] : ["SyncProject" as RecoveryCapability]),
      ...(primaryAction ? [primaryAction.capability] : []),
    ],
    decision_options: [],
    state_fingerprint: `fingerprint-${kind}`,
    ...options,
  };
}

describe("AutopilotControlBar recovery presentation", () => {
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

  function render(recovery: RecoveryPresentation | null, projectValue = project()) {
    const handlers = {
      toggle: vi.fn(async () => undefined),
      sync: vi.fn(async () => undefined),
      acknowledge: vi.fn(async () => undefined),
      retryGit: vi.fn(async () => undefined),
      resume: vi.fn(async () => undefined),
      regenerate: vi.fn(async () => undefined),
      prepare: vi.fn(async () => undefined),
      refresh: vi.fn(async () => undefined),
      runRecovery: vi.fn(async () => undefined),
      resolve: vi.fn(async () => undefined),
      noop: vi.fn(async () => undefined),
    };
    act(() => {
      root.render(
        <AutopilotControlBar
          project={projectValue}
          recoveryPresentation={recovery}
          executionStatus={null}
          busy={false}
          onToggle={handlers.toggle}
          onStopManagedFlow={handlers.noop}
          onPauseNow={handlers.noop}
          onPauseAfterCurrent={handlers.noop}
          onResume={handlers.resume}
          onSync={handlers.sync}
          onAcknowledgeRecovery={handlers.acknowledge}
          onRegeneratePlan={handlers.regenerate}
          onPrepareWorkspace={handlers.prepare}
          onRefreshWorkspace={handlers.refresh}
          onRetryGitConfirmation={handlers.retryGit}
          onRunAutomaticRecovery={handlers.runRecovery}
          onResolveHumanRecovery={handlers.resolve}
        />,
      );
    });
    return handlers;
  }

  it.each([
    ["BaselineRecovery", "AcknowledgeExecutionRecovery", "预览并恢复执行基线"],
    ["GitReconfirmation", "RetryGitConfirmation", "重新确认提交"],
    ["EngineBlocked", "AcknowledgeExecutionRecovery", "检查引擎并重试"],
    ["EvidenceInsufficient", "ResolveHumanRecovery", "补充证据并重新验证"],
    ["HumanDecision", "ResolveHumanRecovery", "选择处理方式"],
  ] as const)("renders one backend-controlled primary action for %s", (kind, capability, label) => {
    const value = project();
    if (capability === "ResolveHumanRecovery") {
      value.workflow_state.recovery_state = {} as NonNullable<
        Project["workflow_state"]["recovery_state"]
      >;
    }
    render(presentation(kind, action(capability, label)), value);
    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());

    expect(host.querySelector(`[data-recovery-kind='${kind}']`)).not.toBeNull();
    expect(buttons).toEqual(["同步状态", label]);
  });

  it("shows validation retry as automatic without inventing a recovery button", () => {
    render(presentation("ValidationRetry", null, {
      title: "等待验证重试",
      automatic_retry: true,
      background_retry_active: true,
      background_retry_summary: "后台重试进行中",
    }));
    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(host.textContent).toContain("后台重试进行中");
    expect(buttons).toEqual(["同步状态"]);
  });

  it("renders no recovery UI for None", () => {
    render(presentation("None", null));
    expect(host.querySelector("[data-recovery-kind]")).toBeNull();
    expect(host.textContent).toContain("已暂停");
  });

  it("uses capability routing and never substitutes baseline recovery for Git", () => {
    const handlers = render(presentation(
      "GitReconfirmation",
      action("RetryGitConfirmation", "重新确认提交"),
    ));
    const button = [...host.querySelectorAll("button")]
      .find(item => item.textContent?.includes("重新确认提交"));
    act(() => button?.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(handlers.retryGit).toHaveBeenCalledTimes(1);
    expect(handlers.acknowledge).not.toHaveBeenCalled();
    const buttons = [...host.querySelectorAll("button")].map(item => item.textContent ?? "");
    expect(buttons.some(label => label.includes("恢复执行基线"))).toBe(false);
  });

  it("shows the backend disabled reason for an unavailable action", () => {
    const primary = action("RetryGitConfirmation", "等待人工核对 Git");
    primary.enabled = false;
    primary.disabled_reason = "系统不会覆盖不可变标签";
    render(presentation("GitReconfirmation", primary));

    const button = [...host.querySelectorAll("button")]
      .find(item => item.textContent?.includes("等待人工核对 Git"));
    expect(button?.hasAttribute("disabled")).toBe(true);
    expect(host.textContent).toContain("系统不会覆盖不可变标签");
  });

  it("opens backend decision options instead of guessing a resolution", async () => {
    const value = project();
    value.workflow_state.recovery_state = {} as NonNullable<
      Project["workflow_state"]["recovery_state"]
    >;
    const handlers = render(presentation(
      "HumanDecision",
      action("ResolveHumanRecovery", "选择处理方式"),
      {
        decision_options: [{
          resolution: "revalidate",
          label: "重新验证",
          enabled: true,
          disabled_reason: null,
          requires_reason: false,
          requires_acceptance_selection: false,
          requires_baseline_preview: false,
        }],
      },
    ), value);
    const openButton = [...host.querySelectorAll("button")]
      .find(item => item.textContent?.includes("选择处理方式"));
    act(() => openButton?.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(document.body.textContent).toContain("重新验证");
    const confirm = [...document.body.querySelectorAll("button")]
      .find(item => item.textContent?.includes("确认处理"));
    await act(async () => {
      confirm?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(handlers.resolve).toHaveBeenCalledWith(
      "revalidate",
      "",
      [],
    );
  });

  it("does not render a human recovery entry when recovery_state is absent", () => {
    render(presentation(
      "HumanDecision",
      action("ResolveHumanRecovery", "选择处理方式"),
      {
        decision_options: [{
          resolution: "revalidate",
          label: "重新验证",
          enabled: true,
          disabled_reason: null,
          requires_reason: false,
          requires_acceptance_selection: false,
          requires_baseline_preview: false,
        }],
      },
    ));

    expect(host.textContent).not.toContain("选择处理方式");
    expect(host.querySelector("[role='dialog']")).toBeNull();
  });

  it("routes stale control lock cleanup through backend sync only", () => {
    const view = presentation(
      "ControlActionOccupied",
      action("ClearStaleControlLock", "清理陈旧锁并恢复操作"),
      {
        secondary_actions: [],
        capabilities: ["ClearStaleControlLock"],
        control_lock_valid: false,
        control_lock_cleanup_available: true,
        control_action_description: "执行任务 · 动作 action-1",
        control_action_elapsed_seconds: 20,
        control_lock_failure_reason: "控制动作心跳已超时",
      },
    );
    const handlers = render(view);
    const buttons = [...host.querySelectorAll("button")];
    expect(buttons.map(button => button.textContent?.trim())).toEqual([
      "清理陈旧锁并恢复操作",
    ]);
    act(() => buttons[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(handlers.sync).toHaveBeenCalledTimes(1);
    expect(host.textContent).toContain("失效原因：控制动作心跳已超时");
  });

  it("keeps normal runtime target, retry, validation, heartbeat, and quality telemetry", () => {
    const value = project();
    const autopilot = value.workflow_state.autopilot_state!;
    autopilot.current_action_kind = "execute_current_subtask";
    autopilot.transient_retry_count = 2;
    autopilot.next_retry_at = "2026-07-31T05:00:00Z";
    autopilot.heartbeat_at = "2026-07-31T04:59:59Z";
    value.execution_session = {
      subtask_id: "task-1",
      verification_stage: "AutomatedTests",
    } as Project["execution_session"];
    value.milestones = [{
      id: "milestone-1",
      title: "同步稳定性",
      subtasks: [],
      mid_stages: [{
        subtasks: [{
          id: "task-1",
          child_tasks: [],
          acceptance_ledger: [],
          test_result: {
            passed: true,
            issues: [],
            suggestion: "",
            automated_test_status: "Passed",
          },
        }],
      }],
    }] as unknown as Project["milestones"];

    render(null, value);
    expect(host.textContent).toContain("目标：同步稳定性");
    expect(host.textContent).toContain("当前：执行当前任务");
    expect(host.textContent).toContain("重试 2/3");
    expect(host.textContent).toContain("验证阶段：运行自动化测试");
    expect(host.textContent).toContain("心跳");
    expect(host.textContent).toContain("自动化测试：通过");
  });

  it("keeps managed-flow target, action, heartbeat, and detail visible", () => {
    const value = project();
    value.workflow_state.autopilot_active = false;
    value.workflow_state.managed_flow_state = {
      active: true,
      managed_state: "running",
      managed_target: "MilestoneSelection",
      last_action: "正在生成首个大阶段",
      last_action_at: "2026-07-31T05:00:00Z",
      run_status: "Running",
      error_message: "",
      job_id: "job-1",
      job_generation: 1,
      current_action: "generate_milestone_draft",
      current_action_id: "action-1",
      heartbeat_at: "2026-07-31T05:00:00Z",
      retry_count: 0,
      last_completed_action: "",
    };

    render(null, value);
    expect(host.textContent).toContain("目标：完成首个大阶段批准");
    expect(host.textContent).toContain("动作：生成大阶段草稿");
    expect(host.textContent).toContain("心跳：");
    expect(host.textContent).toContain("正在生成首个大阶段");
  });
});
