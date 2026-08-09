/* @vitest-environment happy-dom */

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("../utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
}));

vi.mock("./IconButton", () => ({
  IconButton: ({ tooltip, onClick }: { tooltip: string; onClick?: () => void }) => (
    <button type="button" aria-label={tooltip} onClick={onClick}>{tooltip}</button>
  ),
}));

vi.mock("./Modal", () => ({
  Modal: ({
    isOpen,
    children,
    actions = [],
  }: {
    isOpen: boolean;
    children: ReactNode;
    actions?: Array<{ label: string; onClick: () => void; disabled?: boolean }>;
  }) => (
    isOpen ? (
      <div data-testid="settings-modal">
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
import type { AppSettingsView, EngineRuntimeSelfTestResult, Project } from "../types";
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

async function flushPromises() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
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

  async function openModels(view: AppSettingsView, currentProject?: Project) {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(view);
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    act(() => root.render(<ApplicationSettings project={currentProject} />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    act(() => findButton("模型服务").click());
  }

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

    act(() => findButton("运行时自检").click());
    await flushPromises();

    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledWith(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    expect(document.body.textContent).toContain(result.message);
    expect(invokeMock).toHaveBeenCalledWith(
      "test_grok_build_runtime",
      undefined,
      55_000,
    );
    unsubscribe();
  });

  it("does not notify when the self-test request returns no result", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_app_settings") return Promise.resolve(settingsView());
      if (command === "test_grok_build_runtime") return Promise.reject(new Error("自检请求失败"));
      return Promise.reject(new Error(`未预期命令：${command}`));
    });
    const listener = vi.fn();
    const unsubscribe = subscribeEngineHealthInvalidation(listener);
    await openBuiltInTab();

    act(() => findButton("运行时自检").click());
    await flushPromises();

    expect(listener).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("自检请求失败");
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
    await openModels(settingsView(), project(true));
    const visionHeading = [...document.body.querySelectorAll("h3")]
      .find((heading) => heading.textContent?.includes("视觉模型"));
    const visionSection = visionHeading?.closest(".settings-form");
    if (!visionSection) throw new Error("缺少视觉模型设置区域");

    expect(visionSection.textContent).toContain("仅系统凭据库（keyring）");
    expect(visionSection.querySelector('option[value="SessionOnly"]')).toBeNull();
    expect(visionSection.textContent).toContain("不读取环境变量");
  });

  it.each([
    [false, true, "应用级视觉开关已关闭"],
    [true, false, "项目级视觉开关已关闭"],
  ] as const)(
    "blocks the visual capability entry when app=%s and project=%s",
    async (appEnabled, projectEnabled, reason) => {
      const view = settingsView();
      view.settings.vision_model.enabled = appEnabled;
      await openModels(view, project(projectEnabled));

      expect(findButton("用微型 PNG 测试视觉能力").disabled).toBe(true);
      expect(document.body.textContent).toContain(reason);
    },
  );

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
    await openModels(view, project(true));

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
});
