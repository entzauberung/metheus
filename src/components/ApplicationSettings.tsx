import { useEffect, useMemo, useRef, useState } from "react";
import {
  Eye,
  EyeOff,
  FlaskConical,
  RefreshCw,
  RotateCcw,
  Settings2,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import {
  AppSettingsData,
  AppSettingsView,
  ConnectionTestResult,
  EngineAuthenticationResult,
  EngineHealth,
  EngineRuntimeSelfTestResult,
  ExecutionProfile,
  ExecutionProvider,
  ModelConnectionTarget,
  PipelineState,
  Project,
  SecretPersistence,
} from "../types";
import { invokeWithTimeout } from "../utils/invokeWithTimeout";
import { IconButton } from "./IconButton";
import { Modal } from "./Modal";

type SettingsTab = "decision" | "builtin" | "plugins";
type SecretMutation =
  | { action: "Unchanged" }
  | { action: "Replace"; value: string; persistence: SecretPersistence }
  | { action: "Clear" };

interface Props {
  project?: Project | null;
  pipeline?: PipelineState | null;
  className?: string;
}

const TABS: Array<{ id: SettingsTab; label: string }> = [
  { id: "decision", label: "决策模型" },
  { id: "builtin", label: "预装 Grok Build" },
  { id: "plugins", label: "本机插件" },
];

const PLUGIN_PROVIDERS: ExecutionProvider[] = ["ClaudeCode", "Codex", "KimiCli", "GrokBuild"];
const PLUGIN_LABELS: Record<ExecutionProvider, string> = {
  ClaudeCode: "Claude Code",
  Codex: "Codex",
  KimiCli: "Kimi CLI",
  GrokBuild: "Grok Build CLI（本机）",
};

function pluginProfile(provider: ExecutionProvider): ExecutionProfile {
  return {
    runtime: "Plugin",
    provider,
    permission_profile: "Unattended",
    profile_revision: 2,
  };
}

function localAuthLabel(result?: EngineAuthenticationResult): string {
  if (result?.local_state === "ConfiguredEvidence") return "已发现配置";
  if (result?.local_state === "Missing") return "未发现配置";
  return "未知";
}

function onlineAuthLabel(result?: EngineAuthenticationResult): string {
  if (result?.online_state === "Verified") return "已验证";
  if (result?.online_state === "Failed") return "验证失败";
  return "尚未验证";
}

function changeBlockedReason(project?: Project | null, pipeline?: PipelineState | null): string {
  if (pipeline?.status === "Running") return "执行正在运行，暂时不能修改应用设置";
  const recovery = project?.workflow_state.recovery_state;
  if (recovery && ["Diagnosing", "Repairing", "Retesting", "Replanning"].includes(recovery.phase)) {
    return "错误恢复正在进行，暂时不能修改应用设置";
  }
  return "";
}

function secretMutation(
  value: string,
  clear: boolean,
  persistence: SecretPersistence,
): SecretMutation {
  if (clear) return { action: "Clear" };
  if (value.trim()) return { action: "Replace", value, persistence };
  return { action: "Unchanged" };
}

function numberValue(value: string): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function ApplicationSettings({ project, pipeline, className }: Props) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<SettingsTab>("decision");
  const [view, setView] = useState<AppSettingsView | null>(null);
  const [draft, setDraft] = useState<AppSettingsData | null>(null);
  const [decisionSecret, setDecisionSecret] = useState("");
  const [grokSecret, setGrokSecret] = useState("");
  const [clearDecisionSecret, setClearDecisionSecret] = useState(false);
  const [clearGrokSecret, setClearGrokSecret] = useState(false);
  const [showDecisionSecret, setShowDecisionSecret] = useState(false);
  const [showGrokSecret, setShowGrokSecret] = useState(false);
  const [decisionSecretPersistence, setDecisionSecretPersistence] =
    useState<SecretPersistence>("SecureStore");
  const [grokSecretPersistence, setGrokSecretPersistence] =
    useState<SecretPersistence>("SecureStore");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState<ModelConnectionTarget | null>(null);
  const [runtimeTesting, setRuntimeTesting] = useState(false);
  const [runtimeResult, setRuntimeResult] = useState<EngineRuntimeSelfTestResult | null>(null);
  const [pluginHealth, setPluginHealth] = useState<Partial<Record<ExecutionProvider, EngineHealth>>>({});
  const [pluginChecking, setPluginChecking] = useState(false);
  const [verifyingProvider, setVerifyingProvider] = useState<ExecutionProvider | null>(null);
  const [error, setError] = useState("");
  const [connectionResult, setConnectionResult] = useState<ConnectionTestResult | null>(null);
  const requestId = useRef(0);
  const blockedReason = changeBlockedReason(project, pipeline);

  const dirty = useMemo(() => {
    if (!view || !draft) return false;
    return JSON.stringify(view.settings) !== JSON.stringify(draft)
      || Boolean(decisionSecret.trim())
      || Boolean(grokSecret.trim())
      || clearDecisionSecret
      || clearGrokSecret;
  }, [view, draft, decisionSecret, grokSecret, clearDecisionSecret, clearGrokSecret]);

  const resetSecrets = () => {
    setDecisionSecret("");
    setGrokSecret("");
    setClearDecisionSecret(false);
    setClearGrokSecret(false);
    setShowDecisionSecret(false);
    setShowGrokSecret(false);
    setDecisionSecretPersistence("SecureStore");
    setGrokSecretPersistence("SecureStore");
  };

  const close = () => {
    if (saving || testing || runtimeTesting || verifyingProvider) return;
    requestId.current += 1;
    resetSecrets();
    setConnectionResult(null);
    setRuntimeResult(null);
    setError("");
    setOpen(false);
  };

  const applyView = (next: AppSettingsView) => {
    setView(next);
    setDraft(structuredClone(next.settings));
    resetSecrets();
  };

  useEffect(() => {
    if (!open) return;
    const currentRequest = ++requestId.current;
    setLoading(true);
    setError("");
    setConnectionResult(null);
    invokeWithTimeout<AppSettingsView>("get_app_settings")
      .then((next) => {
        if (requestId.current === currentRequest) applyView(next);
      })
      .catch((loadError) => {
        if (requestId.current === currentRequest) setError(String(loadError));
      })
      .finally(() => {
        if (requestId.current === currentRequest) setLoading(false);
      });
  }, [open]);

  useEffect(() => {
    if (!open || tab !== "plugins" || !view) return;
    let active = true;
    setPluginChecking(true);
    Promise.all(
      PLUGIN_PROVIDERS.map(async (provider) => {
        const health = await invokeWithTimeout<EngineHealth>(
          "check_engine_health",
          { executionProfile: pluginProfile(provider) },
        );
        return [provider, health] as const;
      }),
    )
      .then((entries) => {
        if (active) setPluginHealth(Object.fromEntries(entries));
      })
      .catch((healthError) => {
        if (active) setError(String(healthError));
      })
      .finally(() => {
        if (active) setPluginChecking(false);
      });
    return () => { active = false; };
  }, [open, tab, view?.settings.revision]);

  const saveDraft = async (): Promise<AppSettingsView> => {
    if (!view || !draft) throw new Error("应用设置尚未加载");
    const next = await invokeWithTimeout<AppSettingsView>("update_app_settings", {
      expectedRevision: view.settings.revision,
      settings: {
        decision_model: draft.decision_model,
        built_in_grok_build: draft.built_in_grok_build,
        plugin_cli: draft.plugin_cli,
      },
      decisionSecretUpdate: secretMutation(
        decisionSecret,
        clearDecisionSecret,
        decisionSecretPersistence,
      ),
      builtInGrokBuildSecretUpdate: secretMutation(
        grokSecret,
        clearGrokSecret,
        grokSecretPersistence,
      ),
    });
    applyView(next);
    return next;
  };

  const handleSave = async () => {
    if (!dirty || blockedReason) return;
    setSaving(true);
    setError("");
    setConnectionResult(null);
    try {
      await saveDraft();
    } catch (saveError) {
      setError(String(saveError));
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async (target: ModelConnectionTarget) => {
    if (!draft || blockedReason) return;
    setTesting(target);
    setError("");
    setConnectionResult(null);
    try {
      const saved = dirty ? await saveDraft() : view;
      if (!saved) throw new Error("应用设置尚未加载");
      const timeoutSeconds = target === "DecisionModel"
        ? saved.settings.decision_model.timeout_secs
        : saved.settings.built_in_grok_build.timeout_secs;
      const result = await invokeWithTimeout<ConnectionTestResult>(
        "test_model_connection",
        { target },
        (timeoutSeconds + 10) * 1000,
      );
      setConnectionResult(result);
    } catch (testError) {
      setError(String(testError));
    } finally {
      setTesting(null);
    }
  };

  const handleRuntimeTest = async () => {
    if (!draft || blockedReason) return;
    setRuntimeTesting(true);
    setError("");
    setRuntimeResult(null);
    try {
      const saved = dirty ? await saveDraft() : view;
      if (!saved) throw new Error("应用设置尚未加载");
      const result = await invokeWithTimeout<EngineRuntimeSelfTestResult>(
        "test_grok_build_runtime",
        undefined,
        (saved.settings.built_in_grok_build.timeout_secs + 10) * 1000,
      );
      setRuntimeResult(result);
    } catch (testError) {
      setError(String(testError));
    } finally {
      setRuntimeTesting(false);
    }
  };

  const handleVerifyPlugin = async (provider: ExecutionProvider) => {
    if (!draft || blockedReason || !["KimiCli", "GrokBuild"].includes(provider)) return;
    setVerifyingProvider(provider);
    setError("");
    try {
      if (dirty) await saveDraft();
      await invokeWithTimeout<EngineAuthenticationResult>(
        "verify_engine_authentication",
        { executionProfile: pluginProfile(provider) },
      );
      const health = await invokeWithTimeout<EngineHealth>(
        "check_engine_health",
        { executionProfile: pluginProfile(provider) },
      );
      setPluginHealth((current) => ({ ...current, [provider]: health }));
    } catch (verifyError) {
      setError(String(verifyError));
    } finally {
      setVerifyingProvider(null);
    }
  };

  const updateDecision = (change: Partial<AppSettingsData["decision_model"]>) => {
    setDraft((current) => current ? {
      ...current,
      decision_model: { ...current.decision_model, ...change },
    } : current);
  };

  const updateBuiltIn = (change: Partial<AppSettingsData["built_in_grok_build"]>) => {
    setDraft((current) => current ? {
      ...current,
      built_in_grok_build: { ...current.built_in_grok_build, ...change },
    } : current);
  };

  const updatePlugin = (field: keyof AppSettingsData["plugin_cli"], value: string) => {
    setDraft((current) => current ? {
      ...current,
      plugin_cli: { ...current.plugin_cli, [field]: value || undefined },
    } : current);
  };

  const busy = loading
    || saving
    || testing !== null
    || runtimeTesting
    || pluginChecking
    || verifyingProvider !== null;

  return (
    <>
      <IconButton
        icon={<Settings2 size={16} />}
        tooltip="应用设置"
        size="sm"
        className={className}
        onClick={() => setOpen(true)}
      />
      <Modal
        isOpen={open}
        onClose={close}
        title="应用设置"
        lockClose={busy}
        isSubmitting={saving}
        actions={[
          { label: "关闭", onClick: close, variant: "secondary", disabled: busy },
          { label: saving ? "保存中..." : "保存", onClick: handleSave, variant: "primary", disabled: !dirty || busy || Boolean(blockedReason) },
        ]}
      >
        <div className="application-settings">
          <div className="settings-tabs" role="tablist" aria-label="应用设置分类">
            {TABS.map((item) => (
              <button
                type="button"
                role="tab"
                aria-selected={tab === item.id}
                className={tab === item.id ? "selected" : ""}
                key={item.id}
                onClick={() => { setTab(item.id); setConnectionResult(null); setRuntimeResult(null); }}
              >
                {item.label}
              </button>
            ))}
          </div>

          {loading && <div className="settings-state">正在读取设置...</div>}
          {!loading && draft && view && tab === "decision" && (
            <div className="settings-form" role="tabpanel">
              <label>请求地址<input type="url" value={draft.decision_model.request_url} disabled={busy} onChange={(event) => updateDecision({ request_url: event.target.value })} /></label>
              <label>模型名称<input value={draft.decision_model.model} disabled={busy} onChange={(event) => updateDecision({ model: event.target.value })} /></label>
              <div className="settings-grid-two">
                <label>请求超时（秒）<input type="number" min={5} max={3600} value={draft.decision_model.timeout_secs} disabled={busy} onChange={(event) => updateDecision({ timeout_secs: numberValue(event.target.value) })} /></label>
                <label>结构化输出<select value={draft.decision_model.structured_output} disabled={busy} onChange={(event) => updateDecision({ structured_output: event.target.value as AppSettingsData["decision_model"]["structured_output"] })}><option value="NativeJsonObject">原生 JSON</option><option value="PromptOnly">提示词兼容</option></select></label>
              </div>
              <div className="settings-secret-row">
                <label>API Key<span className="settings-secret-hint">{clearDecisionSecret ? "保存后清除" : view.decision_secret.hint}</span><span className="settings-secret-input"><input type={showDecisionSecret ? "text" : "password"} autoComplete="off" value={decisionSecret} disabled={busy || clearDecisionSecret} placeholder={view.decision_secret.configured ? "保持不变" : "输入 API Key"} onChange={(event) => setDecisionSecret(event.target.value)} /><button type="button" title={showDecisionSecret ? "隐藏密钥" : "显示密钥"} disabled={busy || clearDecisionSecret} onClick={() => setShowDecisionSecret((shown) => !shown)}>{showDecisionSecret ? <EyeOff size={15} /> : <Eye size={15} />}</button></span></label>
                <button type="button" className="settings-command" disabled={busy || (!clearDecisionSecret && !view.decision_secret.configured && !decisionSecret)} onClick={() => { setDecisionSecret(""); setClearDecisionSecret((clear) => !clear); }}>{clearDecisionSecret ? <RotateCcw size={14} /> : <Trash2 size={14} />} {clearDecisionSecret ? "撤销" : "清除"}</button>
              </div>
              <label>密钥保存方式<select value={decisionSecretPersistence} disabled={busy || clearDecisionSecret || !decisionSecret.trim()} onChange={(event) => setDecisionSecretPersistence(event.target.value as SecretPersistence)}><option value="SecureStore">安全保存到系统凭据库</option><option value="SessionOnly">仅本次会话</option></select>{view.decision_secret.persistence_error && <span className="settings-secret-hint">{view.decision_secret.persistence_error}</span>}</label>
              <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason)} onClick={() => handleTest("DecisionModel")}><FlaskConical size={15} /> {testing === "DecisionModel" ? "测试中..." : "保存并测试"}</button>
            </div>
          )}

          {!loading && draft && view && tab === "builtin" && (
            <div className="settings-form" role="tabpanel">
              <label>接口后端<select value={draft.built_in_grok_build.api_backend} disabled={busy} onChange={(event) => updateBuiltIn({ api_backend: event.target.value as AppSettingsData["built_in_grok_build"]["api_backend"] })}><option value="ChatCompletions">Chat Completions</option><option value="Responses">Responses</option><option value="Messages">Messages</option></select></label>
              <label>接口地址<input type="url" value={draft.built_in_grok_build.api_base_url} disabled={busy} onChange={(event) => updateBuiltIn({ api_base_url: event.target.value })} /></label>
              <label>模型名称<input value={draft.built_in_grok_build.model} disabled={busy} onChange={(event) => updateBuiltIn({ model: event.target.value })} /></label>
              <div className="settings-grid-two">
                <label>请求超时（秒）<input type="number" min={5} max={3600} value={draft.built_in_grok_build.timeout_secs} disabled={busy} onChange={(event) => updateBuiltIn({ timeout_secs: numberValue(event.target.value) })} /></label>
                <label>最大执行轮数<input type="number" min={1} max={500} value={draft.built_in_grok_build.max_turns} disabled={busy} onChange={(event) => updateBuiltIn({ max_turns: numberValue(event.target.value) })} /></label>
              </div>
              <div className="settings-secret-row">
                <label>API Key<span className="settings-secret-hint">{clearGrokSecret ? "保存后清除" : view.built_in_grok_build_secret.hint}</span><span className="settings-secret-input"><input type={showGrokSecret ? "text" : "password"} autoComplete="off" value={grokSecret} disabled={busy || clearGrokSecret} placeholder={view.built_in_grok_build_secret.configured ? "保持不变" : "输入 API Key"} onChange={(event) => setGrokSecret(event.target.value)} /><button type="button" title={showGrokSecret ? "隐藏密钥" : "显示密钥"} disabled={busy || clearGrokSecret} onClick={() => setShowGrokSecret((shown) => !shown)}>{showGrokSecret ? <EyeOff size={15} /> : <Eye size={15} />}</button></span></label>
                <button type="button" className="settings-command" disabled={busy || (!clearGrokSecret && !view.built_in_grok_build_secret.configured && !grokSecret)} onClick={() => { setGrokSecret(""); setClearGrokSecret((clear) => !clear); }}>{clearGrokSecret ? <RotateCcw size={14} /> : <Trash2 size={14} />} {clearGrokSecret ? "撤销" : "清除"}</button>
              </div>
              <label>密钥保存方式<select value={grokSecretPersistence} disabled={busy || clearGrokSecret || !grokSecret.trim()} onChange={(event) => setGrokSecretPersistence(event.target.value as SecretPersistence)}><option value="SecureStore">安全保存到系统凭据库</option><option value="SessionOnly">仅本次会话</option></select>{view.built_in_grok_build_secret.persistence_error && <span className="settings-secret-hint">{view.built_in_grok_build_secret.persistence_error}</span>}</label>
              <div className="settings-test-actions">
                <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason)} onClick={() => handleTest("BuiltInGrokBuild")}><FlaskConical size={15} /> {testing === "BuiltInGrokBuild" ? "测试中..." : "测试模型连接"}</button>
                <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason)} onClick={handleRuntimeTest}><ShieldCheck size={15} /> {runtimeTesting ? "自检中..." : "运行时自检"}</button>
              </div>
            </div>
          )}

          {!loading && draft && tab === "plugins" && (
            <div className="settings-form" role="tabpanel">
              <label>Claude Code 路径<input value={draft.plugin_cli.claude_code_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("claude_code_path", event.target.value)} /></label>
              <label>Codex 路径<input value={draft.plugin_cli.codex_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("codex_path", event.target.value)} /></label>
              <label>Kimi CLI 路径<input value={draft.plugin_cli.kimi_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("kimi_path", event.target.value)} /></label>
              <label>Grok Build CLI 路径<input value={draft.plugin_cli.grok_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("grok_path", event.target.value)} /></label>
              <div className="plugin-health-list" aria-live="polite">
                {PLUGIN_PROVIDERS.map((provider) => {
                  const health = pluginHealth[provider];
                  const supportsVerification = provider === "KimiCli" || provider === "GrokBuild";
                  return (
                    <div className="plugin-health-row" key={provider}>
                      <div className="plugin-health-name">{PLUGIN_LABELS[provider]}</div>
                      <dl className="plugin-health-facts">
                        <div><dt>安装</dt><dd>{health?.status === "NotInstalled" ? "未安装" : health ? "已发现" : "检查中"}</dd></div>
                        <div><dt>能力</dt><dd>{health?.status === "UnsupportedVersion" ? "不兼容" : health?.capabilities.length ? "已验证" : "未知"}</dd></div>
                        <div><dt>本地配置</dt><dd>{localAuthLabel(health?.authentication)}</dd></div>
                        <div><dt>在线认证</dt><dd>{onlineAuthLabel(health?.authentication)}</dd></div>
                      </dl>
                      {supportsVerification && (
                        <button
                          type="button"
                          className="settings-command plugin-verify"
                          disabled={busy || Boolean(blockedReason) || health?.status === "NotInstalled"}
                          onClick={() => handleVerifyPlugin(provider)}
                        >
                          <RefreshCw size={14} className={verifyingProvider === provider ? "engine-health-spinner" : ""} />
                          {verifyingProvider === provider ? "验证中..." : "在线验证"}
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {view?.load_warning && <div className="settings-warning">{view.load_warning}</div>}
          {blockedReason && <div className="settings-warning">{blockedReason}</div>}
          {connectionResult && <div className={`settings-connection ${connectionResult.success ? "success" : "failure"}`}>{connectionResult.message}{connectionResult.success ? ` · ${connectionResult.latency_ms} ms` : ""}</div>}
          {runtimeResult && <div className={`settings-connection ${runtimeResult.success ? "success" : "failure"}`}>{runtimeResult.message} · {runtimeResult.source_revision.slice(0, 8)}</div>}
          {error && <div className="project-entry-error">{error}</div>}
        </div>
      </Modal>
    </>
  );
}
