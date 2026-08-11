/* @vitest-environment happy-dom */

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("../utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
  decisionModelInvokeTimeoutMs: (timeoutSecs: number) => (
    Math.min(Math.max(timeoutSecs, 0) * 3, 3_600) + 10
  ) * 1_000,
}));

vi.mock("./IconButton", () => ({
  IconButton: ({ tooltip, onClick }: { tooltip: string; onClick?: () => void }) => (
    <button type="button" aria-label={tooltip} onClick={onClick}>{tooltip}</button>
  ),
}));

vi.mock("./Modal", () => ({
  Modal: ({
    isOpen,
    onClose,
    lockClose = false,
    children,
    actions = [],
  }: {
    isOpen: boolean;
    onClose: () => void;
    lockClose?: boolean;
    children: ReactNode;
    actions?: Array<{ label: string; onClick: () => void; disabled?: boolean }>;
  }) => (
    isOpen ? (
      <div data-testid="settings-modal" data-lock-close={String(lockClose)}>
        <button
          aria-label="关闭设置弹窗"
          disabled={lockClose}
          onClick={onClose}
          type="button"
        >
          X
        </button>
        {children}
        {actions.map((action) => (
          <button disabled={action.disabled} key={action.label} onClick={action.onClick} type="button">
            {action.label}
          </button>
        ))}
      </div>
    ) : null
  ),
}));

import {
  BUILT_IN_GROK_BUILD_HEALTH_TARGET,
  subscribeEngineHealthInvalidation,
} from "../engineHealthSync";
import type { AppSettingsView, ConnectionTestResult, EngineHealth, EngineRuntimeSelfTestResult, ExecutionProvider, Project } from "../types";
import { ApplicationSettings } from "./ApplicationSettings";

function settingsView(): AppSettingsView {
  const secret = {
    configured: true,
    source: "Session" as const,
    hint: "已配置",
    persistent_available: true,
    persisted: false,
  };
  return {
    settings: {
      schema_version: 3,
      revision: 7,
      decision_model: {
        api_interface: "OpenAiCompatible",
        request_url: "https://example.test/v1/chat/completions",
        model: "decision-model",
        timeout_secs: 120,
        structured_output: "NativeJsonObject",
      },
      built_in_grok_build: {
        api_backend: "Responses",
        api_base_url: "https://example.test/v1",
        model: "grok-model",
        timeout_secs: 45,
        max_turns: 8,
      },
      plugin_cli: {},
      vision_model: {
        enabled: false,
        request_url: "https://example.test/v1/chat/completions",
        model: "vision-model",
        timeout_secs: 120,
        max_image_bytes: 5_242_880,
        max_total_bytes: 15_728_640,
        max_images: 6,
      },
    },
    decision_secret: secret,
    built_in_grok_build_secret: secret,
    vision_model_secret: {
      configured: true,
      source: "SystemCredentialStore",
      hint: "由系统凭据库提供",
      persistent_available: true,
      persisted: true,
    },
  };
}

function project(visionReviewEnabled: boolean): Project {
  return {
    name: "visual-project",
    human_review_cadence: "MilestoneBatch",
    vision_review_enabled: visionReviewEnabled,
    workflow_state: {
      data_revision: 4,
    },
  } as Project;
}

function runtimeResult(success: boolean): EngineRuntimeSelfTestResult {
  return {
    success,
    state: success ? "Passed" : "Failed",
    source_revision: "abcdef1234567890",
    verified_at: "2026-08-06T00:00:00Z",
    message: success ? "运行时自检通过" : "运行时自检失败",
  };
}

function findButton(label: string): HTMLButtonElement {
  const button = [...document.body.querySelectorAll("button")]
    .find((item) => item.textContent?.trim() === label);
  if (!button) throw new Error(`找不到按钮：${label}`);
  return button;
}

function findSettingsSection(title: string): HTMLElement {
  const heading = [...document.body.querySelectorAll("h3")]
    .find((candidate) => candidate.textContent?.includes(title));
  const section = heading?.closest<HTMLElement>(".settings-form");
  if (!section) throw new Error(`缺少设置区域：${title}`);
  return section;
}

async function flushPromises() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function findEnabledButton(label: string): Promise<HTMLButtonElement> {
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const button = findButton(label);
    if (!button.disabled) return button;
    await flushPromises();
  }
  throw new Error(`按钮在等待后仍不可用：${label}`);
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("ApplicationSettings runtime self-test health invalidation", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    invokeMock.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  async function openBuiltInTab() {
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    act(() => findButton("模型服务").click());
  }

  async function openModels(view: AppSettingsView, currentProject?: Project, target: "decision" | "vision" = "decision") {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(view);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings project={currentProject} />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    act(() => findButton("模型服务").click());
    if (target === "vision") act(() => findButton("视觉模型").click());
  }

  it("keeps the close entry available while initial settings are loading", async () => {
    const pendingSettings = deferred<AppSettingsView>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return pendingSettings.promise;
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());

    const modal = document.body.querySelector('[data-testid="settings-modal"]');
    const closeButton = document.body.querySelector<HTMLButtonElement>('[aria-label="关闭设置弹窗"]');
    expect(modal?.getAttribute("data-lock-close")).toBe("false");
    expect(closeButton?.disabled).toBe(false);
    act(() => closeButton?.click());
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();

    pendingSettings.resolve(settingsView());
    await flushPromises();
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();
  });

  it("closes while plugin health checks continue in the background", async () => {
    const pendingHealth = deferred<{
      status: "Available";
      capabilities: string[];
      authentication: Record<string, never>;
    }>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") return pendingHealth.promise;
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();

    const modal = document.body.querySelector('[data-testid="settings-modal"]');
    const closeButton = document.body.querySelector<HTMLButtonElement>('[aria-label="关闭设置弹窗"]');
    expect(invokeMock.mock.calls.filter(([command]) => command === "check_engine_health")).toHaveLength(4);
    expect(modal?.getAttribute("data-lock-close")).toBe("false");
    expect(closeButton?.disabled).toBe(false);
    act(() => closeButton?.click());
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();

    pendingHealth.resolve({ status: "Available", capabilities: [], authentication: {} });
    await flushPromises();
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();
  });

  it("separates configured evidence from runnable and refreshes matching consumers after explicit verification", async () => {
    let verified = false;
    const pluginHealth = (provider: ExecutionProvider): EngineHealth => ({
      runtime: "Plugin",
      provider,
      status: verified ? "Available" : "VerificationRequired",
      auth_state: verified ? "Authenticated" : "Unknown",
      authentication: {
        local_state: "ConfiguredEvidence",
        online_state: verified ? "Verified" : "NotVerified",
        method: verified ? "OnlineMinimalRequest" : "PassiveConfiguration",
        runtime_configuration: verified ? {
          model: "claude-sonnet-4",
          model_source: "Confirmed",
          reasoning_effort: "high",
          reasoning_effort_source: "Confirmed",
        } : undefined,
        message: verified ? "在线认证通过" : "已发现本地配置",
      },
      supports_unattended: true,
      configuration_valid: true,
      capabilities: verified ? ["unattended"] : [],
      runtime_self_test: "NotRun",
      message: verified ? "插件健康检查通过" : "需要在线验证",
    });
    invokeMock.mockImplementation((command: string, args?: {
      executionProfile?: { provider: ExecutionProvider };
    }) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") {
        return Promise.resolve(pluginHealth(args?.executionProfile?.provider ?? "ClaudeCode"));
      }
      if (command === "verify_engine_authentication") {
        verified = true;
        return Promise.resolve({
          local_state: "ConfiguredEvidence",
          online_state: "Verified",
          method: "OnlineMinimalRequest",
          message: "在线认证通过",
        });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    const listener = vi.fn();
    const unsubscribe = subscribeEngineHealthInvalidation(listener);
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    const firstRow = document.body.querySelector<HTMLElement>(".plugin-health-row");
    if (!firstRow) throw new Error("缺少插件健康行");

    expect(firstRow.textContent).toContain("已发现配置");
    expect(firstRow.textContent).toContain("待在线验证");
    expect(firstRow.textContent).not.toContain("可执行");
    const verifyButton = firstRow.querySelector<HTMLButtonElement>(".plugin-verify");
    await act(async () => verifyButton?.click());
    await flushPromises();

    expect(invokeMock.mock.calls.some(([command]) => command === "verify_engine_authentication")).toBe(true);
    expect(listener).toHaveBeenCalledWith({ runtime: "Plugin", provider: "ClaudeCode" });
    expect(firstRow.textContent).toContain("可执行");
    expect(firstRow.textContent).toContain("重新在线验证");
    expect(firstRow.textContent).toContain("当前模型");
    expect(firstRow.textContent).toContain("claude-sonnet-4");
    expect(firstRow.textContent).toContain("思考强度");
    expect(firstRow.textContent).toContain("high");
    expect(firstRow.textContent).not.toContain("安装");
    expect(firstRow.textContent).not.toContain("在线认证");
    unsubscribe();
  });

  it("falls back to unpublished CLI defaults for legacy successful health", async () => {
    invokeMock.mockImplementation((command: string, args?: {
      executionProfile?: { provider: ExecutionProvider };
    }) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") {
        return Promise.resolve({
          runtime: "Plugin",
          provider: args?.executionProfile?.provider ?? "ClaudeCode",
          status: "Available",
          auth_state: "Authenticated",
          authentication: {
            local_state: "ConfiguredEvidence",
            online_state: "Verified",
            method: "OnlineMinimalRequest",
            message: "verified",
          },
          supports_unattended: true,
          configuration_valid: true,
          capabilities: ["unattended"],
          runtime_self_test: "NotRun",
          message: "verified",
        } satisfies EngineHealth);
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();

    const firstRow = document.body.querySelector<HTMLElement>(".plugin-health-row");
    expect(firstRow?.textContent).toContain("当前模型");
    expect(firstRow?.textContent).toContain("思考强度");
    expect(firstRow?.textContent?.match(/CLI 默认（未公开）/g)).toHaveLength(2);
  });

  it("shows verification progress and a structured failure without exposing the raw error", async () => {
    const pendingVerification = deferred<{
      local_state: "ConfiguredEvidence";
      online_state: "Verified";
      method: "OnlineMinimalRequest";
      message: string;
    }>();
    let verificationStarted = false;
    const pluginHealth = (provider: ExecutionProvider): EngineHealth => ({
      runtime: "Plugin",
      provider,
      status: verificationStarted ? "VerificationFailed" : "VerificationRequired",
      auth_state: "Unknown",
      authentication: {
        local_state: "ConfiguredEvidence",
        online_state: verificationStarted ? "Failed" : "NotVerified",
        method: verificationStarted ? "OnlineMinimalRequest" : "PassiveConfiguration",
        failure_kind: verificationStarted ? "AuthenticationError" : undefined,
        message: verificationStarted ? "在线认证失败 token=backend-secret-value" : "已发现本地配置",
      },
      supports_unattended: true,
      configuration_valid: true,
      capabilities: [],
      runtime_self_test: "NotRun",
      message: verificationStarted ? "认证失败 token=backend-secret-value" : "需要在线验证",
    });
    invokeMock.mockImplementation((command: string, args?: {
      executionProfile?: { provider: ExecutionProvider };
    }) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") {
        return Promise.resolve(pluginHealth(args?.executionProfile?.provider ?? "ClaudeCode"));
      }
      if (command === "verify_engine_authentication") {
        verificationStarted = true;
        return pendingVerification.promise;
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    const firstRow = document.body.querySelector<HTMLElement>(".plugin-health-row");
    const verifyButton = firstRow?.querySelector<HTMLButtonElement>(".plugin-verify");
    if (!firstRow || !verifyButton) throw new Error("缺少插件验证入口");

    act(() => verifyButton.click());
    expect(verifyButton.textContent).toContain("验证中");
    pendingVerification.reject(new Error("provider token=raw-secret-value"));
    await flushPromises();

    expect(firstRow.textContent).toContain("验证失败");
    expect(firstRow.textContent).toContain("查看原因并重试");
    expect(document.body.textContent).toContain("在线验证未完成");
    expect(firstRow.textContent).toContain("原因");
    expect(firstRow.textContent).toContain("认证失败");
    expect(document.body.textContent).not.toContain("raw-secret-value");
    expect(document.body.textContent).not.toContain("backend-secret-value");
  });

  it("requires an explicit save, discard, or continue choice for a dirty draft", async () => {
    vi.useFakeTimers();
    const initial = settingsView();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(initial);
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const modelInput = document.body.querySelector<HTMLInputElement>('input[value="decision-model"]');
    if (!modelInput) throw new Error("缺少决策模型名称输入框");
    act(() => setInputValue(modelInput, "discarded-local-draft"));

    act(() => findButton("关闭").click());
    expect(document.body.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(findButton("保存并关闭")).not.toBeNull();
    expect(findButton("放弃更改")).not.toBeNull();
    expect(findButton("继续编辑")).not.toBeNull();

    act(() => findButton("继续编辑").click());
    expect(document.body.querySelector('[role="alertdialog"]')).toBeNull();
    act(() => findButton("关闭").click());
    act(() => findButton("放弃更改").click());

    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();
    expect(invokeMock.mock.calls.filter(([command]) => command === "update_app_settings")).toHaveLength(0);
  });

  it("allows closing during an atomic save and reloads backend facts on the next open", async () => {
    vi.useFakeTimers();
    const initial = settingsView();
    const saved = settingsView();
    saved.settings.revision = 8;
    saved.settings.decision_model.model = "background-saved";
    const pendingSave = deferred<AppSettingsView>();
    let reads = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") {
        reads += 1;
        return Promise.resolve(reads === 1 ? initial : saved);
      }
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "update_app_settings") return pendingSave.promise;
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const modelInput = document.body.querySelector<HTMLInputElement>('input[value="decision-model"]');
    if (!modelInput) throw new Error("缺少决策模型名称输入框");
    act(() => setInputValue(modelInput, "background-saved"));
    await act(async () => {
      vi.advanceTimersByTime(700);
      await Promise.resolve();
    });
    expect(invokeMock.mock.calls.filter(([command]) => command === "update_app_settings")).toHaveLength(1);

    act(() => findButton("关闭").click());
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();
    act(() => findButton("应用设置").click());
    await flushPromises();
    expect(reads).toBe(1);

    pendingSave.resolve(saved);
    await flushPromises();
    expect(reads).toBe(2);
    expect(document.body.querySelector<HTMLInputElement>('input[value="background-saved"]')).not.toBeNull();
  });

  it("ignores a connection-test response after close", async () => {
    const pendingTest = deferred<ConnectionTestResult>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "test_model_connection") return pendingTest.promise;
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => window.dispatchEvent(new Event("metheus:open-decision-settings")));
    await flushPromises();
    const testButton = await findEnabledButton("保存并测试");
    await act(async () => testButton.click());
    expect(invokeMock.mock.calls.some(([command]) => command === "test_model_connection")).toBe(true);

    act(() => document.body.querySelector<HTMLButtonElement>('[aria-label="关闭设置弹窗"]')?.click());
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();
    pendingTest.resolve({
      success: true,
      target: "DecisionModel",
      model: "decision-model",
      latency_ms: 9,
      message: "stale-connection-result",
    });
    await flushPromises();

    act(() => findButton("应用设置").click());
    await flushPromises();
    expect(document.body.textContent).not.toContain("stale-connection-result");
  });

  it("shows a decision connection result inside its module instead of the page footer", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "test_model_connection") {
        return Promise.resolve({
          success: true,
          target: "DecisionModel",
          model: "decision-model",
          latency_ms: 12,
          message: "决策连接成功",
        });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => window.dispatchEvent(new Event("metheus:open-decision-settings")));
    await flushPromises();
    const testButton = await findEnabledButton("保存并测试");

    await act(async () => testButton.click());
    await flushPromises();

    expect(findSettingsSection("决策模型").textContent).toContain("决策连接成功");
    expect(document.body.querySelector(".application-settings > .settings-connection")).toBeNull();
  });

  it.each([true, false])("notifies BuiltIn Grok exactly once after a %s self-test result", async (success) => {
    const result = runtimeResult(success);
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "test_grok_build_runtime") return Promise.resolve(result);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    const listener = vi.fn();
    const unsubscribe = subscribeEngineHealthInvalidation(listener);
    await openBuiltInTab();
    act(() => findButton("内置 Grok Build").click());

    act(() => findButton("运行时自检").click());
    await flushPromises();

    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledWith(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    expect(document.body.textContent).toContain(result.message);
    expect(findSettingsSection("内置 Grok Build").textContent).toContain(result.message);
    expect(document.body.querySelector(".application-settings > .settings-connection")).toBeNull();
    expect(invokeMock).toHaveBeenCalledWith(
      "test_grok_build_runtime",
      undefined,
      55_000,
    );
    unsubscribe();
  });

  it("keeps model connection testing diagnostic and does not invalidate engine health", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "test_model_connection") {
        return Promise.resolve({
          success: true,
          target: "BuiltInGrokBuild",
          model: "grok-model",
          latency_ms: 8,
          message: "连接诊断通过",
        } satisfies ConnectionTestResult);
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    const listener = vi.fn();
    const unsubscribe = subscribeEngineHealthInvalidation(listener);
    await openBuiltInTab();
    act(() => findButton("内置 Grok Build").click());

    expect(findSettingsSection("内置 Grok Build").textContent).toContain("模型连接测试仅用于诊断接口与凭据");
    expect(findSettingsSection("内置 Grok Build").textContent).toContain("运行时自检是内置引擎的可执行门禁");
    act(() => findButton("测试模型连接").click());
    await flushPromises();

    expect(listener).not.toHaveBeenCalled();
    expect(findSettingsSection("内置 Grok Build").textContent).toContain("连接诊断通过");
    unsubscribe();
  });

  it("does not notify when the self-test request returns no result", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "test_grok_build_runtime") return Promise.reject(new Error("自检请求失败 token=runtime-secret"));
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    const listener = vi.fn();
    const unsubscribe = subscribeEngineHealthInvalidation(listener);
    await openBuiltInTab();
    act(() => findButton("内置 Grok Build").click());

    act(() => findButton("运行时自检").click());
    await flushPromises();

    expect(listener).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("运行时自检请求失败，请重试。");
    expect(document.body.textContent).not.toContain("runtime-secret");
    unsubscribe();
  });

  it("debounces rapid non-sensitive edits and saves only the final value", async () => {
    vi.useFakeTimers();
    const initial = settingsView();
    invokeMock.mockImplementation((command: string, args?: {
      settings?: AppSettingsView["settings"];
    }) => {
      if (command === "get_app_settings") return Promise.resolve(initial);
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "update_app_settings") {
        return Promise.resolve({
          ...initial,
          settings: {
            ...args?.settings,
            schema_version: 3,
            revision: 8,
          },
        });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const modelInput = document.body.querySelector<HTMLInputElement>('input[value="decision-model"]');
    if (!modelInput) throw new Error("缺少决策模型名称输入框");

    for (const value of ["draft-a", "draft-b", "decision-final"]) {
      act(() => setInputValue(modelInput, value));
    }
    await act(async () => {
      vi.advanceTimersByTime(699);
      await Promise.resolve();
    });
    expect(invokeMock.mock.calls.filter(([command]) => command === "update_app_settings")).toHaveLength(0);

    await act(async () => {
      vi.advanceTimersByTime(1);
      await Promise.resolve();
      await Promise.resolve();
    });
    const saves = invokeMock.mock.calls.filter(([command]) => command === "update_app_settings");
    expect(saves).toHaveLength(1);
    expect(saves[0][1].settings.decision_model.model).toBe("decision-final");
    expect(document.body.textContent).toContain("Saved");
  });

  it("marks edits dirty immediately and blocks close until the pending save completes", async () => {
    vi.useFakeTimers();
    const initial = settingsView();
    const updated = settingsView();
    updated.settings.revision = 8;
    updated.settings.decision_model.model = "pending-close-value";
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(initial);
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "update_app_settings") return Promise.resolve(updated);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const modelInput = document.body.querySelector<HTMLInputElement>('input[value="decision-model"]');
    if (!modelInput) throw new Error("缺少决策模型名称输入框");

    act(() => setInputValue(modelInput, "pending-close-value"));
    expect(document.body.textContent).toContain("有未保存更改");
    act(() => findButton("关闭").click());
    expect(document.body.querySelector('[data-testid="settings-modal"]')).not.toBeNull();
    expect(document.body.textContent).toContain("仍有未保存的设置草稿");

    await act(async () => {
      vi.advanceTimersByTime(700);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(document.body.textContent).toContain("Saved");
    act(() => findButton("关闭").click());
    expect(document.body.querySelector('[data-testid="settings-modal"]')).toBeNull();
  });

  it("stops automatic retries after failure and retries only from the explicit action", async () => {
    vi.useFakeTimers();
    const initial = settingsView();
    const updated = settingsView();
    updated.settings.revision = 8;
    let saves = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(initial);
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "update_app_settings") {
        saves += 1;
        return saves === 1
          ? Promise.reject(new Error("隔离保存失败"))
          : Promise.resolve(updated);
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const modelInput = document.body.querySelector<HTMLInputElement>('input[value="decision-model"]');
    if (!modelInput) throw new Error("缺少决策模型名称输入框");
    act(() => setInputValue(modelInput, "retry-value"));

    await act(async () => {
      vi.advanceTimersByTime(700);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(saves).toBe(1);
    expect(document.body.textContent).toContain("保存失败");
    await act(async () => {
      vi.advanceTimersByTime(5_000);
      await Promise.resolve();
    });
    expect(saves).toBe(1);

    await act(async () => findButton("重试保存").click());
    await flushPromises();
    expect(saves).toBe(2);
    expect(document.body.textContent).toContain("Saved");
  });

  it("syncs the latest revision after conflict while preserving the local draft", async () => {
    vi.useFakeTimers();
    const initial = settingsView();
    const latest = settingsView();
    latest.settings.revision = 8;
    latest.settings.decision_model.model = "external-value";
    let reads = 0;
    let saves = 0;
    invokeMock.mockImplementation((command: string, args?: {
      expectedRevision?: number;
      settings?: AppSettingsView["settings"];
    }) => {
      if (command === "get_app_settings") {
        reads += 1;
        return Promise.resolve(reads === 1 ? initial : latest);
      }
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "update_app_settings") {
        saves += 1;
        if (saves === 1) return Promise.reject(new Error("应用设置修订冲突"));
        return Promise.resolve({
          ...latest,
          settings: {
            ...args?.settings,
            schema_version: 3,
            revision: 9,
          },
        });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const modelInput = document.body.querySelector<HTMLInputElement>('input[value="decision-model"]');
    if (!modelInput) throw new Error("缺少决策模型名称输入框");
    act(() => setInputValue(modelInput, "local-draft-value"));

    await act(async () => {
      vi.advanceTimersByTime(700);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(modelInput.value).toBe("local-draft-value");
    expect(document.body.textContent).toContain("本地草稿仍保留");
    expect(saves).toBe(1);

    await act(async () => findButton("重试保存").click());
    await flushPromises();
    const saveCalls = invokeMock.mock.calls.filter(([command]) => command === "update_app_settings");
    expect(saveCalls[1][1].expectedRevision).toBe(8);
    expect(saveCalls[1][1].settings.decision_model.model).toBe("local-draft-value");
  });

  it("writes a secret on blur and clears the plaintext input after success", async () => {
    const initial = settingsView();
    const updated = settingsView();
    updated.settings.revision = 8;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(initial);
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      if (command === "set_api_secret") return Promise.resolve(updated);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    await openBuiltInTab();
    const secretInput = document.body.querySelector<HTMLInputElement>(
      '.settings-form h3:first-child ~ .settings-secret-row input[type="password"]',
    ) ?? document.body.querySelector<HTMLInputElement>('input[type="password"]');
    if (!secretInput) throw new Error("缺少决策模型密钥输入框");

    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(secretInput, "secret-sentinel");
      secretInput.dispatchEvent(new Event("input", { bubbles: true }));
      secretInput.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => secretInput.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_api_secret", {
      expectedRevision: 7,
      target: "DecisionModel",
      secret: "secret-sentinel",
      persistence: "SecureStore",
    });
    expect(secretInput.value).toBe("");
    expect(document.body.textContent).not.toContain("secret-sentinel");
  });

  it("offers vision credentials only through keyring and never renders SessionOnly in that section", async () => {
    const view = settingsView();
    view.settings.vision_model.enabled = true;
    await openModels(view, project(true), "vision");
    const visionSection = findSettingsSection("视觉模型");

    expect(visionSection.textContent).toContain("仅系统凭据库（keyring）");
    expect(visionSection.querySelector('option[value="SessionOnly"]')).toBeNull();
    expect(visionSection.textContent).toContain("不读取环境变量");
  });

  it("keeps vision technical fields collapsed while the app service is disabled", async () => {
    await openModels(settingsView(), project(true), "vision");
    const visionSection = findSettingsSection("视觉模型");

    expect(visionSection.textContent).toContain("视觉模型服务保持关闭");
    expect(visionSection.textContent).toContain("不会自动采集或发送图片");
    expect(visionSection.querySelector('input[type="url"]')).toBeNull();
    expect([...visionSection.querySelectorAll("button")]
      .some((button) => button.textContent?.includes("用微型 PNG"))).toBe(false);
    const toggle = visionSection.querySelector<HTMLInputElement>('[role="switch"]');
    expect(toggle?.getAttribute("aria-checked")).toBe("false");
    expect(toggle?.disabled).toBe(false);
    expect(visionSection.textContent).toContain("视觉服务已关闭");
  });

  it("exposes enabled vision limits with units and preserves exact values", async () => {
    const view = settingsView();
    view.settings.vision_model.enabled = true;
    await openModels(view, project(true), "vision");
    const visionSection = findSettingsSection("视觉模型");
    const toggle = visionSection.querySelector<HTMLInputElement>('[role="switch"]');

    expect(toggle?.getAttribute("aria-checked")).toBe("true");
    expect(visionSection.textContent).toContain("连接与限制");
    expect(visionSection.textContent).toContain("bytes");
    expect(visionSection.textContent).toContain("最多图片数（张）");
    expect(visionSection.querySelector<HTMLInputElement>('input[value="5242880"]')?.step).toBe("1");
    expect(visionSection.querySelector<HTMLInputElement>('input[value="15728640"]')?.value).toBe("15728640");
    expect(visionSection.querySelector('button')?.disabled).toBe(false);
  });

  it("blocks the visual capability entry when the project switch is disabled", async () => {
    const view = settingsView();
    view.settings.vision_model.enabled = true;
    await openModels(view, project(false), "vision");

    expect(findButton("用微型 PNG 测试视觉能力").disabled).toBe(true);
    expect(document.body.textContent).toContain("项目级视觉开关已关闭");
  });

  it("shows a keyring blocked reason and does not infer availability from another source", async () => {
    const view = settingsView();
    view.settings.vision_model.enabled = true;
    view.vision_model_secret = {
      configured: false,
      source: "Missing",
      hint: "系统凭据库不可用",
      persistent_available: false,
      persisted: false,
      persistence_error: "隔离凭据库已锁定",
    };
    await openModels(view, project(true), "vision");

    expect(findButton("用微型 PNG 测试视觉能力").disabled).toBe(true);
    expect(document.body.textContent).toContain("系统凭据库不可用，视觉调用已阻断：隔离凭据库已锁定");
  });

  it("writes a vision placeholder with SecureStore and clears the uncontrolled input", async () => {
    const initial = settingsView();
    initial.settings.vision_model.enabled = true;
    initial.vision_model_secret = {
      configured: false,
      source: "Missing",
      hint: "未配置",
      persistent_available: true,
      persisted: false,
    };
    const updated = settingsView();
    updated.settings.revision = 8;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(initial);
      if (command === "set_api_secret") return Promise.resolve(updated);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings project={project(true)} />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    act(() => findButton("模型服务").click());
    act(() => findButton("视觉模型").click());
    const visionHeading = [...document.body.querySelectorAll("h3")]
      .find((heading) => heading.textContent?.includes("视觉模型"));
    const input = visionHeading?.closest(".settings-form")
      ?.querySelector<HTMLInputElement>('input[type="password"]');
    if (!input) throw new Error("缺少视觉 keyring 输入框");

    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "test-only-placeholder");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => input.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("set_api_secret", {
      expectedRevision: 7,
      target: "VisionModel",
      secret: "test-only-placeholder",
      persistence: "SecureStore",
    });
    expect(input.value).toBe("");
    expect(document.body.textContent).not.toContain("test-only-placeholder");
  });

  it("describes visual results as auxiliary and exposes no automatic adoption action", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings project={project(true)} />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    act(() => findButton("自动化与确认").click());

    expect(document.body.textContent).toContain("必须由人工逐项确认");
    expect([...document.body.querySelectorAll("button")]
      .some((candidate) => candidate.textContent?.includes("自动采用"))).toBe(false);
  });

  it("keeps schema diagnostics internal while preserving the three product settings tabs", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings project={project(true)} />));
    act(() => findButton("应用设置").click());
    await flushPromises();

    const tabs = [...document.body.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual([
      "执行引擎",
      "自动化与确认",
      "模型服务",
    ]);
    expect(tabs.map((tab) => tab.id)).toEqual([
      "settings-tab-engine",
      "settings-tab-automation",
      "settings-tab-models",
    ]);
    expect(tabs.map((tab) => tab.getAttribute("aria-controls"))).toEqual([
      "settings-panel-engine",
      "settings-panel-automation",
      "settings-panel-models",
    ]);
    tabs.forEach((tab) => {
      const panel = document.getElementById(tab.getAttribute("aria-controls") ?? "");
      expect(panel).not.toBeNull();
      expect(panel?.getAttribute("role")).toBe("tabpanel");
      expect(panel?.getAttribute("aria-labelledby")).toBe(tab.id);
      expect((panel as HTMLElement).hidden).toBe(tab.getAttribute("aria-selected") !== "true");
    });
    expect(tabs.filter((tab) => tab.tabIndex === 0)).toHaveLength(1);
    expect(tabs.filter((tab) => tab.getAttribute("aria-selected") === "true")).toHaveLength(1);
    expect(document.body.textContent).not.toContain("高级设置");
    expect(document.body.textContent).not.toContain("设置 Schema");
    expect(document.body.textContent).not.toContain("当前修订");
  });

  it("keeps the selected settings tab linked to a visible loading panel", async () => {
    const pendingSettings = deferred<AppSettingsView>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return pendingSettings.promise;
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());

    const activeTab = document.querySelector<HTMLButtonElement>(".settings-tabs [role='tab'][aria-selected='true']");
    const activePanel = document.getElementById(activeTab?.getAttribute("aria-controls") ?? "");
    expect(activeTab).not.toBeNull();
    expect(activePanel).not.toBeNull();
    expect((activePanel as HTMLElement).hidden).toBe(false);
    expect(activePanel?.textContent).toContain("正在读取设置");
    expect(activePanel?.getAttribute("aria-labelledby")).toBe(activeTab?.id);

    pendingSettings.resolve(settingsView());
    await flushPromises();
  });

  it("keeps a failed settings load inside the selected visible panel", async () => {
    const loadError = new Error("设置读取失败：测试错误");
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.reject(loadError);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();

    const activeTab = document.querySelector<HTMLButtonElement>(".settings-tabs [role='tab'][aria-selected='true']");
    const activePanel = document.getElementById(activeTab?.getAttribute("aria-controls") ?? "");
    expect(activeTab).not.toBeNull();
    expect(activePanel).not.toBeNull();
    expect((activePanel as HTMLElement).hidden).toBe(false);
    expect(activePanel?.getAttribute("role")).toBe("tabpanel");
    expect(activePanel?.getAttribute("aria-labelledby")).toBe(activeTab?.id);
    expect(activePanel?.querySelector('[role="alert"]')?.textContent).toContain(loadError.message);
    expect(document.querySelectorAll('[role="tabpanel"]:not([hidden])')).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_app_settings")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "check_engine_health")).toHaveLength(0);
  });

  it("moves the primary settings tabs with keyboard without shifting focus to the model subnavigation", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "check_engine_health") {
        return Promise.resolve({ status: "Available", capabilities: [], authentication: {} });
      }
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings project={project(true)} />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_app_settings")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "check_engine_health")).toHaveLength(4);

    const tabs = () => [...document.body.querySelectorAll<HTMLButtonElement>(".settings-tabs [role='tab']")];
    const press = (tab: HTMLButtonElement, key: string) => {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      act(() => {
        tab.dispatchEvent(event);
      });
      return event;
    };
    const expectPrimaryTab = (index: number) => {
      const currentTabs = tabs();
      expect(currentTabs[index].tabIndex).toBe(0);
      expect(currentTabs[index].getAttribute("aria-selected")).toBe("true");
      expect(currentTabs.filter((tab) => tab.tabIndex === 0)).toHaveLength(1);
      expect(currentTabs.filter((tab) => tab.getAttribute("aria-selected") === "true")).toHaveLength(1);
      expect(document.activeElement).toBe(currentTabs[index]);
    };

    act(() => tabs()[0].focus());
    let event = press(tabs()[0], "ArrowLeft");
    expect(event.defaultPrevented).toBe(true);
    expectPrimaryTab(2);

    event = press(tabs()[2], "ArrowRight");
    expect(event.defaultPrevented).toBe(true);
    await flushPromises();
    expectPrimaryTab(0);
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_app_settings")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "check_engine_health")).toHaveLength(8);

    event = press(tabs()[0], "ArrowRight");
    expect(event.defaultPrevented).toBe(true);
    expectPrimaryTab(1);

    event = press(tabs()[1], "ArrowRight");
    expect(event.defaultPrevented).toBe(true);
    await flushPromises();
    expectPrimaryTab(2);
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_app_settings")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "check_engine_health")).toHaveLength(8);

    event = press(tabs()[2], "End");
    expect(event.defaultPrevented).toBe(true);
    expectPrimaryTab(2);

    const modelTabs = [...document.body.querySelectorAll<HTMLButtonElement>(".model-service-navigation [role='tab']")];
    expect(modelTabs.find((tab) => tab.getAttribute("aria-selected") === "true")).not.toBeNull();

    event = press(tabs()[2], "ArrowLeft");
    expect(event.defaultPrevented).toBe(true);
    expectPrimaryTab(1);

    event = press(tabs()[1], "Home");
    expect(event.defaultPrevented).toBe(true);
    await flushPromises();
    expectPrimaryTab(0);
    expect(invokeMock.mock.calls.filter(([command]) => command === "get_app_settings")).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(([command]) => command === "check_engine_health")).toHaveLength(12);

    event = press(tabs()[0], "PageDown");
    expect(event.defaultPrevented).toBe(false);
    expectPrimaryTab(0);
  });

  it("routes the decision-settings event and keeps model panels mutually exclusive", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings />));
    act(() => window.dispatchEvent(new Event("metheus:open-decision-settings")));
    await flushPromises();

    const serviceTabs = [...document.body.querySelectorAll<HTMLButtonElement>('[role="tab"]')]
      .filter((tab) => tab.closest(".model-service-navigation"));
    expect(serviceTabs).toHaveLength(3);
    expect(serviceTabs.find((tab) => tab.textContent?.trim() === "决策模型")?.getAttribute("aria-selected"))
      .toBe("true");
    serviceTabs.forEach((tab) => {
      const panel = document.getElementById(tab.getAttribute("aria-controls") ?? "");
      expect(panel).not.toBeNull();
      expect(panel?.getAttribute("role")).toBe("tabpanel");
      expect(panel?.getAttribute("aria-labelledby")).toBe(tab.id);
      expect((panel as HTMLElement).hidden).toBe(tab.getAttribute("aria-selected") !== "true");
    });
    expect(document.querySelectorAll('[role="tabpanel"]:not([hidden])').length).toBe(2);
    expect(document.activeElement).toBe(serviceTabs.find((tab) => tab.textContent?.trim() === "决策模型"));
    expect(document.querySelector<HTMLInputElement>('input[value="decision-model"]')).not.toBeNull();
    expect(document.querySelector<HTMLInputElement>('input[value="grok-model"]')).toBeNull();

    act(() => serviceTabs.find((tab) => tab.textContent?.trim() === "视觉模型")?.click());
    serviceTabs.forEach((tab) => {
      const panel = document.getElementById(tab.getAttribute("aria-controls") ?? "");
      expect(panel).not.toBeNull();
      expect((panel as HTMLElement).hidden).toBe(tab.getAttribute("aria-selected") !== "true");
    });
    expect(document.querySelectorAll('[role="tabpanel"]:not([hidden])').length).toBe(2);
    expect(document.querySelector<HTMLInputElement>('input[value="decision-model"]')).toBeNull();
    expect(findSettingsSection("视觉模型")).not.toBeNull();
    expect(document.querySelector<HTMLInputElement>('input[value="vision-model"]')).toBeNull();
  });
});
