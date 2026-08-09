import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

import {
  decisionModelInvokeTimeoutMs,
  DEFAULT_TIMEOUT_SECS,
  invokeWithTimeout,
  resolveInvokeTimeout,
} from "./invokeWithTimeout";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("resolveInvokeTimeout", () => {
  it("prefers a complete command policy over runtime alias handling", () => {
    expect(resolveInvokeTimeout("test_grok_build_runtime")).toEqual({
      timeoutMs: 620_000,
      source: "exact",
      usedDefault: false,
    });
  });

  it.each([
    ["generate_execution_plan_runtime", 3_610_000],
    ["generate_milestone_draft_runtime", 3_610_000],
    ["run_error_recovery_runtime", 3_610_000],
  ])("inherits one runtime suffix for %s", (command, timeoutMs) => {
    expect(resolveInvokeTimeout(command)).toEqual({
      timeoutMs,
      source: "runtime-base",
      usedDefault: false,
    });
  });

  it.each([
    ["reconcile_on_startup_runtime", 30_000],
    ["update_execution_profile_runtime", 15_000],
    ["update_human_review_policy_runtime", 15_000],
    ["start_managed_flow_runtime", 15_000],
    ["retry_git_confirmation_runtime", 30_000],
    ["acknowledge_execution_recovery_runtime", 30_000],
  ])("has an explicit base policy for bounded wrapper %s", (command, timeoutMs) => {
    expect(resolveInvokeTimeout(command)).toEqual({
      timeoutMs,
      source: "runtime-base",
      usedDefault: false,
    });
  });

  it("strips only one runtime suffix", () => {
    expect(resolveInvokeTimeout("generate_execution_plan_runtime_runtime")).toEqual({
      timeoutMs: DEFAULT_TIMEOUT_SECS * 1000,
      source: "default",
      usedDefault: true,
    });
  });

  it("keeps explicit task-control budgets ahead of every mapped policy", () => {
    expect(resolveInvokeTimeout("apply_task_control_action_runtime", 900_000)).toEqual({
      timeoutMs: 900_000,
      source: "explicit",
      usedDefault: false,
    });
    expect(resolveInvokeTimeout("set_task_control_mode_runtime", 15_000)).toEqual({
      timeoutMs: 15_000,
      source: "explicit",
      usedDefault: false,
    });
  });

  it("uses and exposes the default only for an unknown command", () => {
    expect(resolveInvokeTimeout("unknown_runtime_command")).toEqual({
      timeoutMs: DEFAULT_TIMEOUT_SECS * 1000,
      source: "default",
      usedDefault: true,
    });
  });

  it("stays above the backend decision-model hard deadline", () => {
    expect(decisionModelInvokeTimeoutMs(120)).toBe(370_000);
    expect(decisionModelInvokeTimeoutMs(3_600)).toBe(3_610_000);
    expect(resolveInvokeTimeout("check_subtask").timeoutMs).toBeGreaterThan(
      decisionModelInvokeTimeoutMs(3_600) - 1,
    );
  });
});

describe("invokeWithTimeout", () => {
  it("warns only when resolution reaches the default fallback", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    await invokeWithTimeout("unknown_runtime_command");
    expect(warn).toHaveBeenCalledOnce();
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("使用默认值 30s"));

    warn.mockClear();
    await invokeWithTimeout("generate_execution_plan_runtime");
    await invokeWithTimeout("test_grok_build_runtime");
    await invokeWithTimeout("set_task_control_mode_runtime", {}, 15_000);
    expect(warn).not.toHaveBeenCalled();
  });
});
