import { describe, expect, it } from "vitest";
import type { EngineHealth, EngineHealthStatus, ExecutionRuntime } from "./types";
import { presentEngineHealth, presentPluginHealth } from "./engineHealthPresentation";

function health(
  status: EngineHealthStatus,
  change: Partial<EngineHealth> = {},
  runtime: ExecutionRuntime = "Plugin",
): EngineHealth {
  return {
    runtime,
    provider: "ClaudeCode",
    status,
    executable_path: "/usr/bin/claude",
    version: "1.2.3",
    auth_state: "Unknown",
    authentication: {
      local_state: "Unknown",
      online_state: "NotVerified",
      method: "None",
      message: "",
    },
    supports_unattended: false,
    configuration_valid: true,
    capabilities: [],
    runtime_self_test: "NotRun",
    message: "后端健康事实",
    ...change,
  };
}

describe("presentPluginHealth", () => {
  it.each([
    ["NotInstalled", "未安装", "danger", "install", "安装或配置路径"],
    ["Unauthenticated", "未认证", "danger", "authenticate", "完成认证"],
    ["UnsupportedVersion", "版本不兼容", "danger", "review", "升级或切换版本"],
    ["Disabled", "当前构建未启用", "neutral", "review", "切换可用模式"],
    ["VerificationRequired", "待在线验证", "warning", "verify", "在线验证"],
    ["VerificationFailed", "验证失败", "danger", "verify", "查看原因并重试"],
    ["Available", "可执行", "success", "ready", "可执行"],
    ["Unknown", "状态未知", "neutral", "review", "重新检查"],
  ] as const)(
    "maps %s to a distinct, actionable presentation",
    (status, label, tone, action, actionLabel) => {
      const result = presentPluginHealth(health(status));
      expect(result).toMatchObject({ label, tone, action, actionLabel });
      expect(result.runnable).toBe(status === "Available");
    },
  );

  it("keeps configured evidence as a non-runnable local substate", () => {
    const result = presentPluginHealth(health("VerificationRequired", {
      authentication: {
        local_state: "ConfiguredEvidence",
        online_state: "NotVerified",
        method: "PassiveConfiguration",
        message: "发现本地登录配置",
      },
    }));

    expect(result.label).toBe("待在线验证");
    expect(result.detail).toContain("本地配置：已发现配置");
    expect(result.detail).toContain("在线验证：尚未验证");
    expect(result.runnable).toBe(false);
  });

  it("defaults missing health to Unknown and never Available", () => {
    expect(presentPluginHealth(undefined)).toMatchObject({
      label: "状态未知",
      action: "review",
      runnable: false,
    });
  });

  it("redacts common secret forms from backend detail", () => {
    const result = presentPluginHealth(health("VerificationFailed", {
      message: "Bearer abc.def token=top-secret sk-1234567890abcdef",
    }));

    expect(result.detail).not.toContain("abc.def");
    expect(result.detail).not.toContain("top-secret");
    expect(result.detail).not.toContain("sk-1234567890abcdef");
    expect(result.detail).toContain("[REDACTED]");
  });

  it.each([
    ["VerificationRequired", "待运行时自检", "self-test", "运行时自检", false],
    ["VerificationFailed", "自检失败", "self-test", "查看原因并重试自检", false],
    ["Available", "可执行", "ready", "可执行", true],
    ["Unknown", "状态未知", "review", "重新检查", false],
  ] as const)(
    "maps BuiltIn %s with runtime self-test terminology",
    (status, label, action, actionLabel, runnable) => {
      const result = presentEngineHealth(health(status, {
        runtime_self_test: status === "Available" ? "Passed" : status === "VerificationFailed" ? "Failed" : "NotRun",
      }, "BuiltIn"));

      expect(result).toMatchObject({ label, action, actionLabel, runnable });
      expect(result.detail).toContain("运行时自检：");
      expect(`${result.label} ${result.summary} ${result.detail} ${result.actionLabel}`).not.toContain("在线验证");
    },
  );

  it.each(["VerificationRequired", "VerificationFailed", "Available", "Unknown"] as const)(
    "keeps Plugin %s on online-verification terminology",
    (status) => {
      const result = presentEngineHealth(health(status));
      expect(result.detail).toContain("在线验证：");
      if (status === "VerificationRequired") {
        expect(result).toMatchObject({ label: "待在线验证", action: "verify", actionLabel: "在线验证" });
      }
      expect(result.runnable).toBe(status === "Available");
    },
  );

  it("uses the requested runtime while health is not available", () => {
    const result = presentEngineHealth(undefined, "BuiltIn");
    expect(result.detail).toContain("运行时自检：尚未运行");
    expect(result.detail).not.toContain("在线验证");
  });
});
