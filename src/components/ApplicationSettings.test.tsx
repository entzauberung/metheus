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
  Modal: ({ isOpen, children }: { isOpen: boolean; children: ReactNode }) => (
    isOpen ? <div>{children}</div> : null
  ),
}));

import {
  BUILT_IN_GROK_BUILD_HEALTH_TARGET,
  subscribeEngineHealthInvalidation,
} from "../engineHealthSync";
import type { AppSettingsView, EngineRuntimeSelfTestResult } from "../types";
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
      schema_version: 2,
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
    },
    decision_secret: secret,
    built_in_grok_build_secret: secret,
  };
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
  });

  async function openBuiltInTab() {
    act(() => root.render(<ApplicationSettings />));
    act(() => findButton("应用设置").click());
    await flushPromises();
    act(() => findButton("预装 Grok Build").click());
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
});
