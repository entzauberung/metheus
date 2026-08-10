import type {
  EngineHealth,
  EngineHealthStatus,
  EngineLocalAuthState,
  EngineOnlineAuthState,
  EngineRuntimeSelfTestState,
  ExecutionRuntime,
} from "./types";

export type PluginHealthTone = "success" | "warning" | "danger" | "neutral";
export type PluginHealthAction =
  | "install"
  | "authenticate"
  | "verify"
  | "self-test"
  | "review"
  | "ready";

export interface PluginHealthPresentation {
  label: string;
  tone: PluginHealthTone;
  summary: string;
  detail: string;
  action: PluginHealthAction;
  actionLabel: string;
  runnable: boolean;
}

interface StatusPresentation {
  label: string;
  tone: PluginHealthTone;
  summary: string;
  action: PluginHealthAction;
  actionLabel: string;
}

const STATUS_PRESENTATION: Record<EngineHealthStatus, StatusPresentation> = {
  Available: {
    label: "可执行",
    tone: "success",
    summary: "后端健康检查已确认该插件可用于执行",
    action: "ready",
    actionLabel: "可执行",
  },
  NotInstalled: {
    label: "未安装",
    tone: "danger",
    summary: "未发现插件可执行程序",
    action: "install",
    actionLabel: "安装或配置路径",
  },
  Unauthenticated: {
    label: "未认证",
    tone: "danger",
    summary: "插件存在，但认证状态阻止执行",
    action: "authenticate",
    actionLabel: "完成认证",
  },
  UnsupportedVersion: {
    label: "版本不兼容",
    tone: "danger",
    summary: "当前插件版本不满足无人值守执行要求",
    action: "review",
    actionLabel: "升级或切换版本",
  },
  Disabled: {
    label: "当前构建未启用",
    tone: "neutral",
    summary: "当前运行模式未编译或未开放该引擎",
    action: "review",
    actionLabel: "切换可用模式",
  },
  VerificationRequired: {
    label: "待在线验证",
    tone: "warning",
    summary: "只发现了本地证据，尚未确认在线认证",
    action: "verify",
    actionLabel: "在线验证",
  },
  VerificationFailed: {
    label: "验证失败",
    tone: "danger",
    summary: "在线验证未通过，当前不可执行",
    action: "verify",
    actionLabel: "查看原因并重试",
  },
  Unknown: {
    label: "状态未知",
    tone: "neutral",
    summary: "尚未获得可用于执行的健康结论",
    action: "review",
    actionLabel: "重新检查",
  },
};

const BUILT_IN_STATUS_PRESENTATION: Partial<Record<EngineHealthStatus, StatusPresentation>> = {
  Available: {
    label: "可执行",
    tone: "success",
    summary: "运行时自检已通过，内置引擎可用于执行",
    action: "ready",
    actionLabel: "可执行",
  },
  VerificationRequired: {
    label: "待运行时自检",
    tone: "warning",
    summary: "尚未完成运行时自检，当前不可执行",
    action: "self-test",
    actionLabel: "运行时自检",
  },
  VerificationFailed: {
    label: "自检失败",
    tone: "danger",
    summary: "运行时自检未通过，当前不可执行",
    action: "self-test",
    actionLabel: "查看原因并重试自检",
  },
  Unknown: {
    label: "状态未知",
    tone: "neutral",
    summary: "尚未获得内置引擎的可执行结论",
    action: "review",
    actionLabel: "重新检查",
  },
};

const LOCAL_AUTH_LABELS: Record<EngineLocalAuthState, string> = {
  ConfiguredEvidence: "已发现配置",
  Missing: "未发现配置",
  Unknown: "未知",
};

const ONLINE_AUTH_LABELS: Record<EngineOnlineAuthState, string> = {
  NotVerified: "尚未验证",
  Verified: "已验证",
  Failed: "验证失败",
};

const RUNTIME_SELF_TEST_LABELS: Record<EngineRuntimeSelfTestState, string> = {
  NotRun: "尚未运行",
  Passed: "已通过",
  Failed: "失败",
};

function redactDetail(message: string): string {
  return message
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]+/gi, "Bearer [REDACTED]")
    .replace(/\bsk-[A-Za-z0-9_-]{8,}/g, "[REDACTED]")
    .replace(/((?:api[_-]?key|token|secret)\s*[:=]\s*)[^\s,;]+/gi, "$1[REDACTED]");
}

function statusPresentation(
  status: EngineHealthStatus | undefined,
  runtime: ExecutionRuntime,
): StatusPresentation {
  const resolvedStatus = status ?? "Unknown";
  if (runtime === "BuiltIn") {
    return BUILT_IN_STATUS_PRESENTATION[resolvedStatus] ?? STATUS_PRESENTATION[resolvedStatus];
  }
  return STATUS_PRESENTATION[resolvedStatus];
}

export function presentEngineHealth(
  health?: EngineHealth | null,
  runtime: ExecutionRuntime = health?.runtime ?? "Plugin",
): PluginHealthPresentation {
  const status = statusPresentation(health?.status, runtime);
  if (!health) {
    return {
      ...status,
      detail: runtime === "BuiltIn"
        ? "本地配置：未知 · 运行时自检：尚未运行"
        : "本地配置：未知 · 在线验证：尚未验证",
      runnable: false,
    };
  }

  const localState = health.authentication?.local_state;
  const onlineState = health.authentication?.online_state;
  const capabilities = health.capabilities ?? [];
  const facts = [`本地配置：${localState ? LOCAL_AUTH_LABELS[localState] : "未知"}`];
  if (runtime === "BuiltIn") {
    facts.push(`运行时自检：${RUNTIME_SELF_TEST_LABELS[health.runtime_self_test ?? "NotRun"]}`);
  } else {
    facts.push(`在线验证：${onlineState ? ONLINE_AUTH_LABELS[onlineState] : "尚未验证"}`);
  }
  if (health.version) facts.push(`版本：${health.version}`);
  if (capabilities.length > 0) facts.push(`能力：${capabilities.join("、")}`);
  const message = redactDetail(health.message?.trim() || health.authentication?.message?.trim() || "");
  if (message) facts.push(message);

  return {
    ...status,
    detail: facts.join(" · "),
    runnable: health.status === "Available",
  };
}

export const presentPluginHealth = presentEngineHealth;
