import { useEffect, useRef, useState } from "react";
import { Bot, CheckCircle2, CircleAlert, LoaderCircle, PlugZap } from "lucide-react";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";
import { EngineHealth, ExecutionProfile, ExecutionProvider } from "../types";

interface Props {
  value: ExecutionProfile;
  onChange: (profile: ExecutionProfile) => void;
  disabled?: boolean;
  onHealthChange?: (state: EngineHealthCheckState) => void;
}

export interface EngineHealthCheckState {
  health: EngineHealth | null;
  checking: boolean;
}

const PROVIDERS: Array<{ provider: ExecutionProvider; label: string }> = [
  { provider: "ClaudeCode", label: "Claude Code" },
  { provider: "Codex", label: "Codex" },
];

function statusClass(health: EngineHealth | null): string {
  if (!health || health.status === "Unknown") return "unknown";
  return health.status === "Available" ? "available" : "unavailable";
}

export function ExecutionEngineSelector({ value, onChange, disabled = false, onHealthChange }: Props) {
  const [health, setHealth] = useState<EngineHealth | null>(null);
  const [checking, setChecking] = useState(false);
  const requestIdRef = useRef(0);

  useEffect(() => {
    onHealthChange?.({ health, checking });
  }, [health, checking, onHealthChange]);

  useEffect(() => {
    const requestId = ++requestIdRef.current;
    setChecking(true);
    setHealth(null);
    invokeWithTimeout<EngineHealth>("check_engine_health", { executionProfile: value })
      .then((result) => {
        if (requestIdRef.current === requestId) setHealth(result);
      })
      .catch(() => {
        if (requestIdRef.current !== requestId) return;
        setHealth({
          provider: value.provider,
          status: "Unknown",
          auth_state: "Unknown",
          supports_unattended: value.runtime === "Plugin",
          message: "暂时无法检查引擎状态",
        });
      })
      .finally(() => {
        if (requestIdRef.current === requestId) setChecking(false);
      });
  }, [value.runtime, value.provider, value.permission_profile]);

  const selectProvider = (provider: ExecutionProvider) => {
    onChange({
      ...value,
      runtime: "Plugin",
      provider,
      permission_profile: "Unattended",
    });
  };

  const selectPluginMode = () => {
    const provider = value.provider === "ClaudeCode" || value.provider === "Codex"
      ? value.provider
      : "ClaudeCode";
    selectProvider(provider);
  };

  return (
    <div className="engine-selector">
      <div className="engine-selector-label">执行模式</div>
      <div className="engine-segmented" role="group" aria-label="执行模式">
        <button
          type="button"
          className={value.runtime === "Plugin" ? "selected" : ""}
          disabled={disabled}
          aria-pressed={value.runtime === "Plugin"}
          onClick={selectPluginMode}
        >
          <PlugZap size={15} /> 插件模式
        </button>
        <button
          type="button"
          className={value.runtime === "BuiltIn" ? "selected" : ""}
          disabled
          title="当前版本暂未启用"
          aria-pressed={value.runtime === "BuiltIn"}
        >
          <Bot size={15} /> 预装引擎
          <span className="engine-soon">暂未启用</span>
        </button>
      </div>

      <div className="engine-selector-label">执行引擎</div>
      <div className="engine-provider-options" role="radiogroup" aria-label="执行引擎">
        {PROVIDERS.map((item) => (
          <button
            type="button"
            role="radio"
            aria-checked={value.provider === item.provider}
            className={value.provider === item.provider ? "selected" : ""}
            key={item.provider}
            disabled={disabled}
            onClick={() => selectProvider(item.provider)}
          >
            {item.label}
          </button>
        ))}
      </div>

      <div className={`engine-health ${statusClass(health)}`} aria-live="polite">
        {checking ? (
          <><LoaderCircle className="engine-health-spinner" size={15} /> 正在检查...</>
        ) : health?.status === "Available" ? (
          <><CheckCircle2 size={15} /> {health.message}{health.version ? ` · ${health.version}` : ""}</>
        ) : (
          <><CircleAlert size={15} /> {health?.message || "尚未检查"}</>
        )}
      </div>
    </div>
  );
}
