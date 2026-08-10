/* @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("../utils/invokeWithTimeout", () => ({
  invokeWithTimeout: invokeMock,
}));

import { invalidateEngineHealth, BUILT_IN_GROK_BUILD_HEALTH_TARGET } from "../engineHealthSync";
import type { EngineHealth, EngineHealthStatus, ExecutionProfile } from "../types";
import { ExecutionEngineSelector } from "./ExecutionEngineSelector";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

function profile(runtime: ExecutionProfile["runtime"] = "BuiltIn"): ExecutionProfile {
  return {
    runtime,
    provider: "GrokBuild",
    permission_profile: "Unattended",
    profile_revision: 1,
  };
}

function health(status: EngineHealthStatus, message: string, runtime: ExecutionProfile["runtime"] = "BuiltIn"): EngineHealth {
  return {
    runtime,
    provider: "GrokBuild",
    status,
    auth_state: "Authenticated",
    authentication: {
      local_state: "ConfiguredEvidence",
      online_state: "Verified",
      method: "OnlineMinimalRequest",
      message: "认证可用",
    },
    supports_unattended: true,
    configuration_valid: true,
    capabilities: [],
    runtime_self_test: status === "Available" ? "Passed" : "NotRun",
    message,
  };
}

async function flushPromises() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("ExecutionEngineSelector health invalidation", () => {
  let host: HTMLDivElement;
  let root: Root | null;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    invokeMock.mockReset();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    if (root) act(() => root?.unmount());
    host.remove();
  });

  function render(value = profile()) {
    act(() => root?.render(
      <ExecutionEngineSelector value={value} onChange={vi.fn()} />,
    ));
  }

  it("rechecks a mounted BuiltIn Grok consumer without reload", async () => {
    invokeMock
      .mockResolvedValueOnce(health("VerificationRequired", "需要运行时自检"))
      .mockResolvedValueOnce(health("Available", "内置引擎可用"));
    render();
    await flushPromises();
    expect(host.textContent).toContain("需要运行时自检");
    expect(host.textContent).toContain("待运行时自检");
    expect(host.textContent).toContain("下一步：运行时自检");
    expect(host.textContent).not.toContain("在线验证");

    act(() => invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET));
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("内置引擎可用");
    expect(host.textContent).toContain("可执行");
  });

  it("presents a BuiltIn self-test failure without online-auth terminology", async () => {
    const failed = health("VerificationFailed", "运行时自检没有通过");
    failed.runtime_self_test = "Failed";
    failed.authentication.online_state = "Failed";
    invokeMock.mockResolvedValue(failed);
    render();
    await flushPromises();

    expect(host.textContent).toContain("自检失败");
    expect(host.textContent).toContain("下一步：查看原因并重试自检");
    expect(host.textContent).not.toContain("在线验证");
  });

  it("explains configured evidence as pending verification rather than runnable", async () => {
    const pending = health("VerificationRequired", "只发现本地认证配置", "Plugin");
    pending.authentication = {
      local_state: "ConfiguredEvidence",
      online_state: "NotVerified",
      method: "PassiveConfiguration",
      message: "发现配置",
    };
    invokeMock.mockResolvedValue(pending);
    render(profile("Plugin"));
    await flushPromises();

    expect(host.textContent).toContain("待在线验证");
    expect(host.textContent).toContain("本地配置：已发现配置");
    expect(host.textContent).toContain("下一步：在线验证");
    expect(host.textContent).not.toContain("后端健康检查已确认该插件可用于执行");
  });

  it("keeps request sequence protection when an old health response arrives last", async () => {
    const oldRequest = deferred<EngineHealth>();
    const newRequest = deferred<EngineHealth>();
    invokeMock
      .mockReturnValueOnce(oldRequest.promise)
      .mockReturnValueOnce(newRequest.promise);
    render();

    act(() => invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET));
    await act(async () => {
      newRequest.resolve(health("Available", "最新健康状态"));
      await newRequest.promise;
    });
    expect(host.textContent).toContain("最新健康状态");

    await act(async () => {
      oldRequest.resolve(health("VerificationRequired", "过期健康状态"));
      await oldRequest.promise;
    });
    expect(host.textContent).toContain("最新健康状态");
    expect(host.textContent).not.toContain("过期健康状态");
  });

  it("ignores the old provider response after the selected provider changes", async () => {
    const grokRequest = deferred<EngineHealth>();
    const codexRequest = deferred<EngineHealth>();
    invokeMock
      .mockReturnValueOnce(grokRequest.promise)
      .mockReturnValueOnce(codexRequest.promise);
    const grokProfile = profile("Plugin");
    render(grokProfile);
    render({ ...grokProfile, provider: "Codex" });

    const codexHealth = health("Available", "Codex 最新状态", "Plugin");
    codexHealth.provider = "Codex";
    await act(async () => {
      codexRequest.resolve(codexHealth);
      await codexRequest.promise;
    });
    expect(host.textContent).toContain("Codex 最新状态");

    await act(async () => {
      grokRequest.resolve(health("VerificationRequired", "Grok 旧状态", "Plugin"));
      await grokRequest.promise;
    });
    expect(host.textContent).toContain("Codex 最新状态");
    expect(host.textContent).not.toContain("Grok 旧状态");
  });

  it("does not recheck Plugin Grok for a BuiltIn invalidation", async () => {
    invokeMock.mockResolvedValue(health("Available", "插件可用", "Plugin"));
    render(profile("Plugin"));
    await flushPromises();

    act(() => invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET));
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("removes the invalidation listener on unmount", async () => {
    invokeMock.mockResolvedValue(health("Available", "内置引擎可用"));
    render();
    await flushPromises();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    act(() => root?.unmount());
    root = null;
    invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    await Promise.resolve();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
