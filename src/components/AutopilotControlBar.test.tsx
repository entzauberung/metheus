/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  PipelineState,
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

  function render(
    recovery: RecoveryPresentation | null,
    projectValue = project(),
    executionStatus: PipelineState | null = null,
  ) {
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
      pauseManaged: vi.fn(async () => undefined),
      resumeManaged: vi.fn(async () => undefined),
      pauseNow: vi.fn(async () => undefined),
      pauseAfterCurrent: vi.fn(async () => undefined),
      noop: vi.fn(async () => undefined),
    };
    act(() => {
      root.render(
        <AutopilotControlBar
          project={projectValue}
          recoveryPresentation={recovery}
          executionStatus={executionStatus}
          busy={false}
          onToggle={handlers.toggle}
          onPauseManagedFlow={handlers.pauseManaged}
          onResumeManagedFlow={handlers.resumeManaged}
          onStopManagedFlow={handlers.noop}
          onPauseNow={handlers.pauseNow}
          onPauseAfterCurrent={handlers.pauseAfterCurrent}
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
    expect(buttons).toEqual([label, "同步状态"]);
  });

  it("shows validation retry as automatic without inventing a recovery button", () => {
    render(presentation("ValidationRetry", null, {
      title: "等待验证重试",
      automatic_retry: true,
      progress_status: "scheduled",
      background_retry_active: true,
      background_retry_summary: "后台重试进行中",
      next_retry_at: "2026-07-31T05:00:00Z",
    }));
    const buttons = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(host.textContent).toContain("后台重试进行中");
    expect(buttons).toEqual(["同步状态"]);
  });

  it.each([
    ["inactive", "恢复未运行"],
    ["queued", "自动恢复已排队"],
    ["scheduled", "恢复重试已安排"],
    ["running", "自动恢复执行中"],
    ["warning", "恢复进展延迟"],
    ["stalled", "恢复已停滞"],
    ["waiting_human", "自动恢复已停止"],
  ] as const)("renders structured %s recovery progress", (status, label) => {
    render(presentation("AutomaticRecovery", null, {
      severity: status === "stalled" || status === "waiting_human" ? "Error" : "Warning",
      progress_status: status,
      current_action: status === "running" || status === "warning" || status === "stalled"
        ? "run_error_recovery"
        : null,
      action_started_at: "2026-07-31T04:55:00Z",
      elapsed_seconds: 305,
      last_progress_at: "2026-07-31T04:56:00Z",
      hard_deadline_at: "2026-07-31T05:07:00Z",
      next_retry_at: status === "scheduled" ? "2026-07-31T05:00:00Z" : null,
      background_retry_active: ["scheduled", "running", "warning", "stalled"].includes(status),
    }));

    const bar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    const statusRegion = bar?.querySelector<HTMLElement>(".ap-bar-status-region");
    expect(bar?.dataset.recoveryProgress).toBe(status);
    expect(statusRegion?.textContent).toContain(label);
    expect(statusRegion?.querySelector("svg")).not.toBeNull();
    expect(statusRegion?.getAttribute("aria-label")).toBe("恢复进展");
    expect(statusRegion?.getAttribute("aria-live")).toBe("polite");
    expect(statusRegion?.getAttribute("aria-atomic")).toBe("true");
    if (status === "stalled") {
      expect(bar?.querySelector(".ap-bar-summary")?.textContent).toContain("恢复质量错误");
      expect(bar?.querySelector(".ap-bar-detail-content")?.textContent).toContain("已持续：5 分 5 秒");
      expect(bar?.querySelector(".ap-bar-detail-content")?.textContent).toContain("最后业务进展");
      expect(bar?.querySelector(".ap-bar-detail-content")?.textContent).toContain("最迟终止");
    }
  });

  it("uses a safe compatibility label for an old DTO without claiming background work", () => {
    render(presentation("AutomaticRecovery", null, {
      automatic_retry: true,
      background_retry_active: true,
      background_retry_summary: "后台重试进行中",
    }));

    expect(host.textContent).toContain("恢复进度未记录");
    expect(host.textContent).not.toContain("后台重试进行中");
    expect(host.querySelector<HTMLElement>(".autopilot-control-bar")?.dataset.recoveryProgress)
      .toBe("unknown");
  });

  it("renders no recovery UI for None", () => {
    render(presentation("None", null));
    expect(host.querySelector("[data-recovery-kind]")).toBeNull();
    expect(host.textContent).toContain("已暂停");
  });

  it("keeps status, summary, and actions regions in a stable order across states", () => {
    const recoveryView = presentation(
      "GitReconfirmation",
      action("RetryGitConfirmation", "重新确认提交"),
      { reason: "一段很长但不能推动主操作区的恢复诊断说明" },
    );
    render(recoveryView);

    const recoveryBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    expect([...recoveryBar!.children].map(child => child.classList[0])).toEqual([
      "ap-bar-status-region",
      "ap-bar-summary-region",
      "ap-bar-actions-region",
    ]);
    expect([...recoveryBar!.querySelectorAll("[data-action-slot]")]
      .map(slot => slot.getAttribute("data-action-slot")))
      .toEqual(["primary", "secondary", "overflow"]);
    expect(recoveryBar?.dataset.apState).toBe("Error");
    expect(recoveryBar?.querySelector(".ap-bar-summary")?.getAttribute("title"))
      .toBe("进度未记录");
    expect(recoveryBar?.querySelector(".ap-bar-details")).not.toBeNull();
    expect(recoveryBar?.querySelector(".ap-bar-detail-content")?.textContent)
      .toContain("恢复诊断说明");

    render(null);
    const pausedBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    expect([...pausedBar!.children].map(child => child.classList[0])).toEqual([
      "ap-bar-status-region",
      "ap-bar-summary-region",
      "ap-bar-actions-region",
    ]);
    expect(pausedBar?.dataset.apState).toBe("Paused");
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
    const labels = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(labels).toEqual(["暂停托管", "停止托管"]);
  });

  it("keeps managed resume in the same command bar without a duplicate pause action", () => {
    const value = project();
    value.workflow_state.autopilot_active = false;
    value.workflow_state.managed_flow_state = {
      active: true,
      managed_state: "paused",
      managed_target: "MilestoneSelection",
      last_action: "等待恢复",
      last_action_at: "2026-07-31T05:00:00Z",
      run_status: "Paused",
      error_message: "",
      job_id: "job-1",
      job_generation: 1,
      current_action: "",
      current_action_id: "",
      heartbeat_at: "2026-07-31T05:00:00Z",
      retry_count: 0,
      last_completed_action: "generate_milestone_draft",
    };

    const handlers = render(null, value);
    const labels = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(labels).toEqual(["恢复托管", "停止托管"]);
    const resume = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("恢复托管"));
    act(() => resume?.click());
    expect(handlers.resumeManaged).toHaveBeenCalledTimes(1);
  });

  it("keeps ErrorStopped managed flow actionable with restart and stop buttons", () => {
    const value = project();
    value.workflow_state.autopilot_active = false;
    value.workflow_state.managed_flow_state = {
      active: true,
      managed_state: "error",
      managed_target: "MilestoneSelection",
      last_action: "托管动作重试耗尽，已停止",
      last_action_at: "2026-07-31T05:00:00Z",
      run_status: "ErrorStopped",
      error_message: "模型连接失败，请检查 API 配置",
      job_id: "job-error-1",
      job_generation: 3,
      current_action: "",
      current_action_id: "",
      heartbeat_at: "2026-07-31T05:00:00Z",
      retry_count: 3,
      last_completed_action: "",
    };

    const handlers = render(null, value);
    const labels = [...host.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(labels).toEqual(["重新启动托管", "停止托管"]);
    expect(host.textContent).toContain("托管层因错误停止");
    expect(host.textContent).toContain("模型连接失败，请检查 API 配置");
    const restart = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("重新启动托管"));
    act(() => restart?.click());
    expect(handlers.resumeManaged).toHaveBeenCalledTimes(1);
  });

  it("preserves the responsive region contract while switching Running, Paused, and Recovery", () => {
    const value = project();
    value.workflow_state.autopilot_state!.run_status = "Running";
    render(null, value);

    const runningBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    expect(runningBar?.dataset.apState).toBe("Running");
    expect(runningBar?.querySelector(".ap-bar-status-region")?.getAttribute("role")).toBe("status");
    expect(runningBar?.querySelector(".ap-bar-summary-region")?.getAttribute("aria-label"))
      .toBe("执行摘要");
    expect(runningBar?.querySelector(".ap-bar-actions-region")?.getAttribute("aria-label"))
      .toBe("自动驾驶操作");
    expect(runningBar?.querySelector(".ap-bar-actions-region")?.textContent)
      .toContain("暂停自动驾驶");

    value.workflow_state.autopilot_state!.run_status = "Paused";
    render(null, value);
    const pausedBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    expect(pausedBar?.dataset.apState).toBe("Paused");
    expect(pausedBar?.querySelector(".ap-bar-actions-region")?.textContent).toContain("恢复");

    render(presentation(
      "GitReconfirmation",
      action("RetryGitConfirmation", "重新确认提交"),
    ));
    const recoveryBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    expect(recoveryBar?.dataset.apState).toBe("Error");
    expect(recoveryBar?.querySelector(".ap-bar-actions-region")?.textContent)
      .toContain("重新确认提交");
    expect([...recoveryBar!.children].map(child => child.classList[0])).toEqual([
      "ap-bar-status-region",
      "ap-bar-summary-region",
      "ap-bar-actions-region",
    ]);
  });

  it("keeps long action, long error, and retry facts in focusable details without moving actions", () => {
    const longAction = "execute_a_very_long_action_name_that_must_not_expand_the_command_track";
    const value = project();
    value.workflow_state.autopilot_state!.run_status = "Running";
    value.workflow_state.autopilot_state!.current_action_kind = longAction;
    value.workflow_state.autopilot_state!.transient_retry_count = 2;
    value.workflow_state.autopilot_state!.next_retry_at = "2026-07-31T05:00:00Z";
    render(null, value);

    const runningBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    expect(runningBar?.querySelector(".ap-bar-summary")?.getAttribute("title"))
      .not.toContain(longAction);
    expect(runningBar?.querySelector(".ap-bar-detail-content")?.textContent)
      .toContain(longAction);
    expect(runningBar?.querySelector(".ap-bar-detail-content")?.textContent).toContain("重试 2/3");
    expect(runningBar?.querySelector(".ap-bar-actions-region")?.textContent)
      .toContain("暂停自动驾驶");

    const longError = "恢复诊断很长，需要保留完整错误信息但不能把核心操作推出窄屏可视区域";
    render(presentation(
      "GitReconfirmation",
      action("RetryGitConfirmation", "重新确认提交"),
      {
        reason: longError,
        retry_count: 2,
        retry_limit: 3,
        next_retry_at: "2026-07-31T05:00:00Z",
      },
    ));

    const recoveryBar = host.querySelector<HTMLElement>(".autopilot-control-bar");
    const details = recoveryBar?.querySelector<HTMLDetailsElement>(".ap-bar-details");
    const detailsSummary = details?.querySelector<HTMLElement>("summary");
    const primaryAction = recoveryBar?.querySelector<HTMLButtonElement>(
      ".ap-bar-actions-region .ap-bar-btn-primary",
    );
    expect(details?.textContent).toContain(longError);
    expect(details?.textContent).toContain("后台重试 2/3");
    expect(primaryAction?.textContent).toContain("重新确认提交");

    act(() => detailsSummary?.focus());
    expect(document.activeElement).toBe(detailsSummary);
    act(() => primaryAction?.focus());
    expect(document.activeElement).toBe(primaryAction);
  });

  it.each([390, 600, 1024, 1280])(
    "keeps fixed regions and action slots at a %dpx viewport contract",
    (width) => {
      host.style.width = `${width}px`;
      const value = project();
      value.workflow_state.autopilot_state!.run_status = "Running";
      render(null, value);

      const bar = host.querySelector<HTMLElement>(".autopilot-control-bar");
      expect(bar?.dataset.actionLayout).toBe("fixed-slots");
      expect(bar?.dataset.detailLayout).toBe("flow-bounded");
      expect([...bar!.children].map(child => child.classList[0])).toEqual([
        "ap-bar-status-region",
        "ap-bar-summary-region",
        "ap-bar-actions-region",
      ]);
      expect([...bar!.querySelectorAll("[data-action-slot]")]
        .map(slot => slot.getAttribute("data-action-slot")))
        .toEqual(["primary", "secondary", "overflow"]);
      expect(bar?.querySelector("[data-action-slot='primary']")?.textContent)
        .toContain("暂停自动驾驶");
    },
  );

  it("keeps all additional recovery actions keyboard reachable in overflow", () => {
    const view = presentation(
      "EngineBlocked",
      action("AcknowledgeExecutionRecovery", "重试执行"),
      {
        secondary_actions: [
          action("SyncProject", "同步状态"),
          action("RefreshExecutionWorkspace", "刷新工作区"),
          action("CloseAutopilot", "关闭自动驾驶"),
        ],
        capabilities: [
          "AcknowledgeExecutionRecovery",
          "SyncProject",
          "RefreshExecutionWorkspace",
          "CloseAutopilot",
        ],
      },
    );
    render(view);

    const primary = host.querySelector<HTMLElement>("[data-action-slot='primary']");
    const secondary = host.querySelector<HTMLElement>("[data-action-slot='secondary']");
    const overflow = host.querySelector<HTMLDetailsElement>(".ap-action-overflow");
    const overflowSummary = overflow?.querySelector<HTMLElement>("summary");
    expect(primary?.textContent).toContain("重试执行");
    expect(secondary?.textContent).toContain("同步状态");
    expect(overflow?.textContent).toContain("刷新工作区");
    expect(overflow?.textContent).toContain("关闭自动驾驶");

    act(() => overflowSummary?.focus());
    expect(document.activeElement).toBe(overflowSummary);
    const refresh = [...overflow!.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("刷新工作区"));
    act(() => refresh?.focus());
    expect(document.activeElement).toBe(refresh);
  });
});
