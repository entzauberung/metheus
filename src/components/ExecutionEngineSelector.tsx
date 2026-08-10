import { useEffect, useRef, useState } from "react";
import { Bot, CheckCircle2, CircleHelp, CircleX, LoaderCircle, PlugZap, ShieldAlert } from "lucide-react";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";
import { EngineHealth, ExecutionProfile, ExecutionProvider } from "../types";
import { PLUGIN_EXECUTION_PROVIDERS } from "../enginePolicy";
import { presentEngineHealth, type PluginHealthTone } from "../engineHealthPresentation";
import {
  matchesEngineHealthTarget,
  subscribeEngineHealthInvalidation,
} from "../engineHealthSync";

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

const PROVIDER_LABELS: Record<ExecutionProvider, string> = {
  ClaudeCode: "Claude Code",
  Codex: "Codex",
  KimiCli: "Kimi CLI",
  GrokBuild: "Grok Build CLI（本机）",
};

function HealthStatusIcon({ tone }: { tone: PluginHealthTone }) {
  if (tone === "success") return <CheckCircle2 size={16} />;
  if (tone === "warning") return <ShieldAlert size={16} />;
  if (tone === "danger") return <CircleX size={16} />;
  return <CircleHelp size={16} />;
}

export function ExecutionEngineSelector({ value, onChange, disabled = false, onHealthChange }: Props) {
  const [health, setHealth] = useState<EngineHealth | null>(null);
  const [checking, setChecking] = useState(false);
  const requestIdRef = useRef(0);

  useEffect(() => {
    onHealthChange?.({ health, checking });
  }, [health, checking, onHealthChange]);

  useEffect(() => {
    const checkHealth = () => {
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
            runtime: value.runtime,
            provider: value.provider,
            status: "Unknown",
            auth_state: "Unknown",
            authentication: {
              local_state: "Unknown",
              online_state: "NotVerified",
              method: "None",
              message: "暂时无法检查认证状态",
            },
            supports_unattended: value.runtime === "Plugin",
            configuration_valid: false,
            capabilities: [],
            runtime_self_test: "NotRun",
            message: "暂时无法检查引擎状态",
          });
        })
        .finally(() => {
          if (requestIdRef.current === requestId) setChecking(false);
        });
    };

    checkHealth();
    const unsubscribe = subscribeEngineHealthInvalidation((target) => {
      if (matchesEngineHealthTarget(value, target)) checkHealth();
    });
    return () => {
      requestIdRef.current += 1;
      unsubscribe();
    };
  }, [value.runtime, value.provider, value.permission_profile]);

  const selectProvider = (provider: ExecutionProvider) => {
    onChange({
      ...value,
      runtime: value.runtime,
      provider,
      permission_profile: "Unattended",
    });
  };

  const selectPluginMode = () => {
    onChange({
      ...value,
      runtime: "Plugin",
      provider: value.provider,
      permission_profile: "Unattended",
    });
  };

  const selectBuiltInMode = () => {
    onChange({
      ...value,
      runtime: "BuiltIn",
      provider: "GrokBuild",
      permission_profile: "Unattended",
    });
  };
  const healthPresentation = presentEngineHealth(health, value.runtime);

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
          disabled={disabled}
          aria-pressed={value.runtime === "BuiltIn"}
          onClick={selectBuiltInMode}
        >
          <Bot size={15} /> 内置模式
        </button>
      </div>

      <div className="engine-selector-label">执行引擎</div>
      <div className="engine-provider-options" role="radiogroup" aria-label="执行引擎">
        {(value.runtime === "BuiltIn"
          ? [{ provider: "GrokBuild" as ExecutionProvider, label: "Grok Build（内置）" }]
          : PLUGIN_EXECUTION_PROVIDERS.map((provider) => ({ provider, label: PROVIDER_LABELS[provider] }))
        ).map((item) => (
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

      <div className={`engine-health tone-${healthPresentation.tone}`} aria-live="polite">
        {checking ? (
          <><LoaderCircle className="engine-health-spinner" size={15} /> 正在检查...</>
        ) : (
          <>
            <HealthStatusIcon tone={healthPresentation.tone} />
            <span className="engine-health-copy">
              <strong>{healthPresentation.label}</strong>
              <span>{healthPresentation.summary}</span>
              <small>{healthPresentation.detail} · 下一步：{healthPresentation.actionLabel}</small>
            </span>
          </>
        )}
      </div>
    </div>
  );
}
