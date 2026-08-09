import { useEffect, useMemo, useRef, useState } from "react";
import {
  Eye,
  EyeOff,
  FlaskConical,
  RefreshCw,
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
  RuntimeMutationResult,
  SecretPersistence,
} from "../types";
import {
  BUILT_IN_GROK_BUILD_HEALTH_TARGET,
  invalidateEngineHealth,
} from "../engineHealthSync";
import {
  decisionModelInvokeTimeoutMs,
  invokeWithTimeout,
} from "../utils/invokeWithTimeout";
import { IconButton } from "./IconButton";
import { Modal } from "./Modal";

type SettingsTab = "engine" | "automation" | "models" | "advanced";
type SecretMutation =
  | { action: "Unchanged" }
  | { action: "Replace"; value: string; persistence: SecretPersistence }
  | { action: "Clear" };

interface Props {
  project?: Project | null;
  pipeline?: PipelineState | null;
  className?: string;
  writeBlockedReason?: string;
  onRuntimeMutation?: (result: RuntimeMutationResult) => void;
}

const TABS: Array<{ id: SettingsTab; label: string }> = [
  { id: "engine", label: "执行引擎" },
  { id: "automation", label: "自动化与确认" },
  { id: "models", label: "模型服务" },
  { id: "advanced", label: "高级设置" },
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

export function ApplicationSettings({ project, pipeline, className, writeBlockedReason = "", onRuntimeMutation }: Props) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<SettingsTab>("engine");
  const [view, setView] = useState<AppSettingsView | null>(null);
  const [draft, setDraft] = useState<AppSettingsData | null>(null);
  const [decisionSecret, setDecisionSecret] = useState("");
  const [grokSecret, setGrokSecret] = useState("");
  const visionSecretRef = useRef<HTMLInputElement>(null);
  const [visionSecretPending, setVisionSecretPending] = useState(false);
  const [clearDecisionSecret, setClearDecisionSecret] = useState(false);
  const [clearGrokSecret, setClearGrokSecret] = useState(false);
  const [showDecisionSecret, setShowDecisionSecret] = useState(false);
  const [showGrokSecret, setShowGrokSecret] = useState(false);
  const [showVisionSecret, setShowVisionSecret] = useState(false);
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
  const [saveState, setSaveState] = useState<"idle" | "dirty" | "saving" | "saved" | "error">("idle");
  const [autoSaveBlocked, setAutoSaveBlocked] = useState(false);
  const requestId = useRef(0);
  const blockedReason = writeBlockedReason || changeBlockedReason(project, pipeline);

  const dirty = useMemo(() => {
    if (!view || !draft) return false;
    return JSON.stringify(view.settings) !== JSON.stringify(draft)
      || Boolean(decisionSecret.trim())
      || Boolean(grokSecret.trim())
      || visionSecretPending
      || clearDecisionSecret
      || clearGrokSecret;
  }, [view, draft, decisionSecret, grokSecret, visionSecretPending, clearDecisionSecret, clearGrokSecret]);
  const nonSensitiveDirty = useMemo(
    () => Boolean(view && draft && JSON.stringify(view.settings) !== JSON.stringify(draft)),
    [view, draft],
  );

  const resetSecrets = () => {
    setDecisionSecret("");
    setGrokSecret("");
    if (visionSecretRef.current) visionSecretRef.current.value = "";
    setVisionSecretPending(false);
    setClearDecisionSecret(false);
    setClearGrokSecret(false);
    setShowDecisionSecret(false);
    setShowGrokSecret(false);
    setShowVisionSecret(false);
    setDecisionSecretPersistence("SecureStore");
    setGrokSecretPersistence("SecureStore");
  };

  const close = () => {
    if (saving) {
      setError("设置正在保存，完成后才能关闭。");
      return;
    }
    if (testing || runtimeTesting || verifyingProvider) return;
    if (dirty) {
      if (saveState !== "error") {
        setError("仍有未保存的设置草稿；请等待自动保存或使用保存按钮重试。");
      }
      if (saveState !== "error") setSaveState("dirty");
      return;
    }
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
    setAutoSaveBlocked(false);
  };

  const markDirty = () => setSaveState((current) => current === "error" ? "error" : "dirty");

  useEffect(() => {
    const openDecisionSettings = () => {
      setTab("models");
      setOpen(true);
    };
    window.addEventListener("metheus:open-decision-settings", openDecisionSettings);
    return () => window.removeEventListener("metheus:open-decision-settings", openDecisionSettings);
  }, []);

  useEffect(() => {
    if (!open) return;
    const currentRequest = ++requestId.current;
    setLoading(true);
    setSaveState("idle");
    setAutoSaveBlocked(false);
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
    if (!open || tab !== "engine" || !view) return;
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
        vision_model: draft.vision_model,
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
      visionModelSecretUpdate: secretMutation(
        visionSecretRef.current?.value ?? "",
        false,
        "SecureStore",
      ),
    });
    applyView(next);
    return next;
  };

  useEffect(() => {
    if (!open || !view || !draft || !nonSensitiveDirty || blockedReason || saving || autoSaveBlocked) return;
    const snapshot = structuredClone(draft);
    const snapshotKey = JSON.stringify(snapshot);
    const expectedRevision = view.settings.revision;
    const timer = window.setTimeout(() => {
      setSaving(true);
      setSaveState("saving");
      setError("");
      invokeWithTimeout<AppSettingsView>("update_app_settings", {
        expectedRevision,
        settings: {
          decision_model: snapshot.decision_model,
          built_in_grok_build: snapshot.built_in_grok_build,
          plugin_cli: snapshot.plugin_cli,
          vision_model: snapshot.vision_model,
        },
        decisionSecretUpdate: { action: "Unchanged" },
        builtInGrokBuildSecretUpdate: { action: "Unchanged" },
        visionModelSecretUpdate: { action: "Unchanged" },
      })
        .then((next) => {
          setView(next);
          setDraft((current) => (
            current && JSON.stringify(current) === snapshotKey
              ? structuredClone(next.settings)
              : current
          ));
          setAutoSaveBlocked(false);
          setSaveState("saved");
        })
        .catch(async (saveError) => {
          const message = String(saveError);
          setAutoSaveBlocked(true);
          setSaveState("error");
          setError(message);
          if (message.includes("修订") || message.includes("磁盘冲突")) {
            try {
              const latest = await invokeWithTimeout<AppSettingsView>("get_app_settings");
              setView(latest);
              setError(`${message}；已同步最新服务端基线，本地草稿仍保留，请确认后重试保存。`);
            } catch {
              // The original conflict remains the actionable error.
            }
          }
        })
        .finally(() => setSaving(false));
    }, 700);
    return () => window.clearTimeout(timer);
  }, [open, view, draft, nonSensitiveDirty, blockedReason, saving, autoSaveBlocked]);

  const handleSave = async () => {
    if (!dirty || blockedReason) return;
    setSaving(true);
    setAutoSaveBlocked(true);
    setSaveState("saving");
    setError("");
    setConnectionResult(null);
    try {
      await saveDraft();
      setAutoSaveBlocked(false);
      setSaveState("saved");
    } catch (saveError) {
      const message = String(saveError);
      setAutoSaveBlocked(true);
      setSaveState("error");
      setError(message);
      if (message.includes("修订") || message.includes("磁盘冲突")) {
        try {
          const latest = await invokeWithTimeout<AppSettingsView>("get_app_settings");
          setView(latest);
          setError(`${message}；已同步最新服务端基线，本地草稿仍保留，请确认后重试保存。`);
        } catch {
          // Keep the original conflict and local draft.
        }
      }
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
        : target === "VisionModel"
          ? saved.settings.vision_model.timeout_secs
          : saved.settings.built_in_grok_build.timeout_secs;
      const invokeTimeoutMs = target === "DecisionModel"
        ? decisionModelInvokeTimeoutMs(timeoutSeconds)
        : (timeoutSeconds + 10) * 1000;
      const result = await invokeWithTimeout<ConnectionTestResult>(
        "test_model_connection",
        { target },
        invokeTimeoutMs,
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
      invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    } catch (testError) {
      setError(String(testError));
    } finally {
      setRuntimeTesting(false);
    }
  };

  const handleVerifyPlugin = async (provider: ExecutionProvider) => {
    if (!draft || blockedReason) return;
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
    markDirty();
    setDraft((current) => current ? {
      ...current,
      decision_model: { ...current.decision_model, ...change },
    } : current);
  };

  const updateBuiltIn = (change: Partial<AppSettingsData["built_in_grok_build"]>) => {
    markDirty();
    setDraft((current) => current ? {
      ...current,
      built_in_grok_build: { ...current.built_in_grok_build, ...change },
    } : current);
  };

  const updatePlugin = (field: keyof AppSettingsData["plugin_cli"], value: string) => {
    markDirty();
    setDraft((current) => current ? {
      ...current,
      plugin_cli: { ...current.plugin_cli, [field]: value || undefined },
    } : current);
  };

  const updateVision = (change: Partial<AppSettingsData["vision_model"]>) => {
    markDirty();
    setDraft((current) => current ? {
      ...current,
      vision_model: { ...current.vision_model, ...change },
    } : current);
  };

  const updateProjectPolicy = async (
    humanReviewCadence: Project["human_review_cadence"],
    visionReviewEnabled: boolean,
  ) => {
    if (!project || blockedReason || busy) return;
    setSaving(true);
    setError("");
    try {
      const result = await invokeWithTimeout<RuntimeMutationResult>(
        "update_human_review_policy_runtime",
        {
          projectName: project.name,
          expectedRevision: project.workflow_state.data_revision,
          humanReviewCadence,
          visionReviewEnabled,
        },
      );
      onRuntimeMutation?.(result);
      setSaveState("saved");
    } catch (policyError) {
      setSaveState("error");
      setError(String(policyError));
    } finally {
      setSaving(false);
    }
  };

  const commitSecret = async (
    target: "DecisionModel" | "BuiltInGrokBuild" | "VisionModel",
    value: string,
    persistence: SecretPersistence,
  ) => {
    if (!view || !value.trim() || blockedReason || busy) return;
    setSaving(true);
    setSaveState("saving");
    setError("");
    try {
      let current = view;
      if (draft && JSON.stringify(current.settings) !== JSON.stringify(draft)) {
        current = await invokeWithTimeout<AppSettingsView>("update_app_settings", {
          expectedRevision: current.settings.revision,
          settings: {
            decision_model: draft.decision_model,
            built_in_grok_build: draft.built_in_grok_build,
            plugin_cli: draft.plugin_cli,
            vision_model: draft.vision_model,
          },
          decisionSecretUpdate: { action: "Unchanged" },
          builtInGrokBuildSecretUpdate: { action: "Unchanged" },
          visionModelSecretUpdate: { action: "Unchanged" },
        });
      }
      const next = await invokeWithTimeout<AppSettingsView>("set_api_secret", {
        expectedRevision: current.settings.revision,
        target,
        secret: value,
        persistence,
      });
      applyView(next);
      setSaveState("saved");
    } catch (secretError) {
      setSaveState("error");
      setError(String(secretError));
    } finally {
      setSaving(false);
    }
  };

  const clearSecret = async (
    target: "DecisionModel" | "BuiltInGrokBuild" | "VisionModel",
  ) => {
    if (!view || blockedReason || busy) return;
    setSaving(true);
    setSaveState("saving");
    setError("");
    try {
      const next = await invokeWithTimeout<AppSettingsView>("clear_api_secret", {
        expectedRevision: view.settings.revision,
        target,
      });
      applyView(next);
      setSaveState("saved");
    } catch (secretError) {
      setSaveState("error");
      setError(String(secretError));
    } finally {
      setSaving(false);
    }
  };

  const busy = loading
    || saving
    || testing !== null
    || runtimeTesting
    || pluginChecking
    || verifyingProvider !== null;
  const visionCredentialReady = Boolean(
    view?.vision_model_secret.configured
      && view.vision_model_secret.persisted
      && view.vision_model_secret.source === "SystemCredentialStore",
  );
  const visionBlockedReason = !draft?.vision_model.enabled
    ? "应用级视觉开关已关闭，视觉调用已阻断。"
    : !project
      ? "未打开项目，视觉调用已阻断。"
      : !project.vision_review_enabled
        ? "项目级视觉开关已关闭，视觉调用已阻断。"
        : !visionCredentialReady
          ? view?.vision_model_secret.persistence_error
            ? `系统凭据库不可用，视觉调用已阻断：${view.vision_model_secret.persistence_error}`
            : "系统凭据库未配置视觉模型 API Key，视觉调用已阻断。"
          : "";

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
          { label: saving ? "保存中..." : saveState === "error" ? "重试保存" : "保存", onClick: handleSave, variant: "primary", disabled: !dirty || busy || Boolean(blockedReason) },
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
          {!loading && project && tab === "automation" && (
            <div className="settings-form" role="tabpanel">
              <h3>人工确认频率</h3>
              <div className="settings-segmented" role="radiogroup" aria-label="人工确认频率">
                <button
                  aria-checked={project.human_review_cadence === "PerTask"}
                  disabled={busy || Boolean(blockedReason)}
                  onClick={() => updateProjectPolicy("PerTask", project.vision_review_enabled)}
                  role="radio"
                  type="button"
                >逐任务确认</button>
                <button
                  aria-checked={project.human_review_cadence === "MilestoneBatch"}
                  disabled={busy || Boolean(blockedReason)}
                  onClick={() => updateProjectPolicy("MilestoneBatch", project.vision_review_enabled)}
                  role="radio"
                  type="button"
                >大阶段集中确认</button>
              </div>
              <p className="settings-hint">
                旧项目会保留逐任务策略；新项目默认在现有 A/B/C 大阶段边界集中确认。
              </p>
              <label className="settings-toggle-row">
                <input
                  checked={project.vision_review_enabled}
                  disabled={busy || Boolean(blockedReason)}
                  onChange={(event) => updateProjectPolicy(project.human_review_cadence, event.target.checked)}
                  type="checkbox"
                />
                使用视觉模型辅助检查任务明确声明的截图
              </label>
              <p className="settings-hint">
                视觉结果（包括问题、证据不足和冲突）只在集中人工确认清单中作为辅助证据显示，必须由人工逐项确认；没有自动采用入口。
              </p>
              {project.vision_review_enabled && visionBlockedReason && (
                <p className="settings-warning" role="alert">{visionBlockedReason}</p>
              )}
            </div>
          )}

          {!loading && !project && tab === "automation" && (
            <div className="settings-state">打开项目后可配置人工确认和视觉辅助策略。</div>
          )}

          {!loading && draft && view && tab === "models" && (
            <div className="settings-form" role="tabpanel">
              <h3>决策模型</h3>
              <label>请求地址<input type="url" value={draft.decision_model.request_url} disabled={busy} onChange={(event) => updateDecision({ request_url: event.target.value })} /></label>
              <label>模型名称<input value={draft.decision_model.model} disabled={busy} onChange={(event) => updateDecision({ model: event.target.value })} /></label>
              <div className="settings-grid-two">
                <label>连接 / 分块空闲超时（秒）<input type="number" min={5} max={3600} value={draft.decision_model.timeout_secs} disabled={busy} onChange={(event) => updateDecision({ timeout_secs: numberValue(event.target.value) })} /></label>
                <p className="settings-hint">持续返回数据时不会按单次空闲时限中断；硬总时限为该值的 3 倍，最长 3600 秒。</p>
                <label>结构化输出<select value={draft.decision_model.structured_output} disabled={busy} onChange={(event) => updateDecision({ structured_output: event.target.value as AppSettingsData["decision_model"]["structured_output"] })}><option value="NativeJsonObject">原生 JSON</option><option value="PromptOnly">提示词兼容</option></select></label>
              </div>
              <div className="settings-secret-row">
                <label>API Key<span className="settings-secret-hint">{view.decision_secret.hint}</span><span className="settings-secret-input"><input type={showDecisionSecret ? "text" : "password"} autoComplete="off" value={decisionSecret} disabled={busy} placeholder={view.decision_secret.configured ? "保持不变" : "输入 API Key"} onBlur={() => commitSecret("DecisionModel", decisionSecret, decisionSecretPersistence)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); commitSecret("DecisionModel", decisionSecret, decisionSecretPersistence); } }} onChange={(event) => { setDecisionSecret(event.target.value); markDirty(); }} /><button type="button" title={showDecisionSecret ? "隐藏密钥" : "显示密钥"} disabled={busy} onClick={() => setShowDecisionSecret((shown) => !shown)}>{showDecisionSecret ? <EyeOff size={15} /> : <Eye size={15} />}</button></span></label>
                <button type="button" className="settings-command" disabled={busy || !view.decision_secret.configured} onClick={() => clearSecret("DecisionModel")}><Trash2 size={14} /> 清除</button>
              </div>
              <label>密钥保存方式<select value={decisionSecretPersistence} disabled={busy || clearDecisionSecret || !decisionSecret.trim()} onChange={(event) => setDecisionSecretPersistence(event.target.value as SecretPersistence)}><option value="SecureStore">安全保存到系统凭据库</option><option value="SessionOnly">仅本次会话</option></select>{view.decision_secret.persistence_error && <span className="settings-secret-hint">{view.decision_secret.persistence_error}</span>}</label>
              <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason)} onClick={() => handleTest("DecisionModel")}><FlaskConical size={15} /> {testing === "DecisionModel" ? "测试中..." : "保存并测试"}</button>
            </div>
          )}

          {!loading && draft && view && tab === "models" && (
            <div className="settings-form" role="tabpanel">
              <h3>内置 Grok Build</h3>
              <label>接口后端<select value={draft.built_in_grok_build.api_backend} disabled={busy} onChange={(event) => updateBuiltIn({ api_backend: event.target.value as AppSettingsData["built_in_grok_build"]["api_backend"] })}><option value="ChatCompletions">Chat Completions</option><option value="Responses">Responses</option><option value="Messages">Messages</option></select></label>
              <label>接口地址<input type="url" value={draft.built_in_grok_build.api_base_url} disabled={busy} onChange={(event) => updateBuiltIn({ api_base_url: event.target.value })} /></label>
              <label>模型名称<input value={draft.built_in_grok_build.model} disabled={busy} onChange={(event) => updateBuiltIn({ model: event.target.value })} /></label>
              <div className="settings-grid-two">
                <label>请求超时（秒）<input type="number" min={5} max={3600} value={draft.built_in_grok_build.timeout_secs} disabled={busy} onChange={(event) => updateBuiltIn({ timeout_secs: numberValue(event.target.value) })} /></label>
                <label>最大执行轮数<input type="number" min={1} max={500} value={draft.built_in_grok_build.max_turns} disabled={busy} onChange={(event) => updateBuiltIn({ max_turns: numberValue(event.target.value) })} /></label>
              </div>
              <div className="settings-secret-row">
                <label>API Key<span className="settings-secret-hint">{view.built_in_grok_build_secret.hint}</span><span className="settings-secret-input"><input type={showGrokSecret ? "text" : "password"} autoComplete="off" value={grokSecret} disabled={busy} placeholder={view.built_in_grok_build_secret.configured ? "保持不变" : "输入 API Key"} onBlur={() => commitSecret("BuiltInGrokBuild", grokSecret, grokSecretPersistence)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); commitSecret("BuiltInGrokBuild", grokSecret, grokSecretPersistence); } }} onChange={(event) => { setGrokSecret(event.target.value); markDirty(); }} /><button type="button" title={showGrokSecret ? "隐藏密钥" : "显示密钥"} disabled={busy} onClick={() => setShowGrokSecret((shown) => !shown)}>{showGrokSecret ? <EyeOff size={15} /> : <Eye size={15} />}</button></span></label>
                <button type="button" className="settings-command" disabled={busy || !view.built_in_grok_build_secret.configured} onClick={() => clearSecret("BuiltInGrokBuild")}><Trash2 size={14} /> 清除</button>
              </div>
              <label>密钥保存方式<select value={grokSecretPersistence} disabled={busy || clearGrokSecret || !grokSecret.trim()} onChange={(event) => setGrokSecretPersistence(event.target.value as SecretPersistence)}><option value="SecureStore">安全保存到系统凭据库</option><option value="SessionOnly">仅本次会话</option></select>{view.built_in_grok_build_secret.persistence_error && <span className="settings-secret-hint">{view.built_in_grok_build_secret.persistence_error}</span>}</label>
              <div className="settings-test-actions">
                <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason)} onClick={() => handleTest("BuiltInGrokBuild")}><FlaskConical size={15} /> {testing === "BuiltInGrokBuild" ? "测试中..." : "测试模型连接"}</button>
                <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason)} onClick={handleRuntimeTest}><ShieldCheck size={15} /> {runtimeTesting ? "自检中..." : "运行时自检"}</button>
              </div>
            </div>
          )}

          {!loading && draft && view && tab === "models" && (
            <div className="settings-form" role="tabpanel">
              <h3>视觉模型（默认关闭）</h3>
              <label className="settings-toggle-row">
                <input
                  checked={draft.vision_model.enabled}
                  disabled={busy}
                  onChange={(event) => updateVision({ enabled: event.target.checked })}
                  type="checkbox"
                />
                启用视觉模型服务
              </label>
              <label>请求地址<input type="url" value={draft.vision_model.request_url} disabled={busy} onChange={(event) => updateVision({ request_url: event.target.value })} /></label>
              <label>模型名称<input value={draft.vision_model.model} disabled={busy} onChange={(event) => updateVision({ model: event.target.value })} /></label>
              <div className="settings-grid-two">
                <label>请求超时（秒）<input type="number" min={5} max={3600} value={draft.vision_model.timeout_secs} disabled={busy} onChange={(event) => updateVision({ timeout_secs: numberValue(event.target.value) })} /></label>
                <label>最多图片数<input type="number" min={1} max={20} value={draft.vision_model.max_images} disabled={busy} onChange={(event) => updateVision({ max_images: numberValue(event.target.value) })} /></label>
                <label>单图上限（字节）<input type="number" min={1} value={draft.vision_model.max_image_bytes} disabled={busy} onChange={(event) => updateVision({ max_image_bytes: numberValue(event.target.value) })} /></label>
                <label>总大小上限（字节）<input type="number" min={1} value={draft.vision_model.max_total_bytes} disabled={busy} onChange={(event) => updateVision({ max_total_bytes: numberValue(event.target.value) })} /></label>
              </div>
              <div className="settings-secret-row">
                <label>API Key<span className="settings-secret-hint">{view.vision_model_secret.hint}</span><span className="settings-secret-input"><input ref={visionSecretRef} type={showVisionSecret ? "text" : "password"} autoComplete="off" disabled={busy || !view.vision_model_secret.persistent_available} placeholder={visionCredentialReady ? "保持不变" : "输入 API Key"} onBlur={(event) => commitSecret("VisionModel", event.currentTarget.value, "SecureStore")} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); commitSecret("VisionModel", event.currentTarget.value, "SecureStore"); } }} onChange={() => { setVisionSecretPending(Boolean(visionSecretRef.current?.value.trim())); markDirty(); }} /><button type="button" title={showVisionSecret ? "隐藏密钥" : "显示密钥"} disabled={busy} onClick={() => setShowVisionSecret((shown) => !shown)}>{showVisionSecret ? <EyeOff size={15} /> : <Eye size={15} />}</button></span></label>
                <button type="button" className="settings-command" disabled={busy || !visionCredentialReady} onClick={() => clearSecret("VisionModel")}><Trash2 size={14} /> 清除</button>
              </div>
              <p className="settings-hint">密钥保存方式：仅系统凭据库（keyring）。不读取环境变量，也不支持仅会话保存。</p>
              {view.vision_model_secret.persistence_error && (
                <p className="settings-warning" role="alert">{view.vision_model_secret.persistence_error}</p>
              )}
              {visionBlockedReason && <p className="settings-warning" role="alert">{visionBlockedReason}</p>}
              <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason) || Boolean(visionBlockedReason)} onClick={() => handleTest("VisionModel")}><FlaskConical size={15} /> {testing === "VisionModel" ? "测试中..." : "用微型 PNG 测试视觉能力"}</button>
            </div>
          )}

          {!loading && draft && tab === "engine" && (
            <div className="settings-form" role="tabpanel">
              <h3>外部 CLI 插件</h3>
              <label>Claude Code 路径<input value={draft.plugin_cli.claude_code_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("claude_code_path", event.target.value)} /></label>
              <label>Codex 路径<input value={draft.plugin_cli.codex_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("codex_path", event.target.value)} /></label>
              <label>Kimi CLI 路径<input value={draft.plugin_cli.kimi_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("kimi_path", event.target.value)} /></label>
              <label>Grok Build CLI 路径<input value={draft.plugin_cli.grok_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("grok_path", event.target.value)} /></label>
              <p className="settings-hint">外部 CLI 始终使用你的本机默认模型和 Provider；Metheus 不注入模型参数。</p>
              <div className="plugin-health-list" aria-live="polite">
                {PLUGIN_PROVIDERS.map((provider) => {
                  const health = pluginHealth[provider];
                  return (
                    <div className="plugin-health-row" key={provider}>
                      <div className="plugin-health-name">{PLUGIN_LABELS[provider]}</div>
                      <dl className="plugin-health-facts">
                        <div><dt>安装</dt><dd>{health?.status === "NotInstalled" ? "未安装" : health ? "已发现" : "检查中"}</dd></div>
                        <div><dt>能力</dt><dd>{health?.status === "UnsupportedVersion" ? "不兼容" : health?.capabilities.length ? "已验证" : "未知"}</dd></div>
                        <div><dt>本地配置</dt><dd>{localAuthLabel(health?.authentication)}</dd></div>
                        <div><dt>在线认证</dt><dd>{onlineAuthLabel(health?.authentication)}</dd></div>
                      </dl>
                      <button
                        type="button"
                        className="settings-command plugin-verify"
                        disabled={busy || Boolean(blockedReason) || health?.status === "NotInstalled"}
                        onClick={() => handleVerifyPlugin(provider)}
                      >
                        <RefreshCw size={14} className={verifyingProvider === provider ? "engine-health-spinner" : ""} />
                        {verifyingProvider === provider ? "验证中..." : "在线验证"}
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {!loading && draft && tab === "advanced" && (
            <div className="settings-form" role="tabpanel">
              <h3>高级设置</h3>
              <dl className="plugin-health-facts">
                <div><dt>设置 Schema</dt><dd>v{draft.schema_version}</dd></div>
                <div><dt>当前修订</dt><dd>{draft.revision}</dd></div>
                <div><dt>保存方式</dt><dd>非敏感配置 700ms 自动保存；密钥写入系统凭据库</dd></div>
              </dl>
              <p className="settings-hint">API Key 不会写入应用设置或项目 JSON。</p>
            </div>
          )}

          {view?.load_warning && <div className="settings-warning">{view.load_warning}</div>}
          {blockedReason && <div className="settings-warning">{blockedReason}</div>}
          {connectionResult && <div className={`settings-connection ${connectionResult.success ? "success" : "failure"}`}>{connectionResult.message}{connectionResult.success ? ` · ${connectionResult.latency_ms} ms` : ""}</div>}
          {runtimeResult && <div className={`settings-connection ${runtimeResult.success ? "success" : "failure"}`}>{runtimeResult.message} · {runtimeResult.source_revision.slice(0, 8)}</div>}
          <div className={`settings-save-state ${saveState}`}>{saveState === "saving" ? "Saving…" : saveState === "error" ? "保存失败" : saveState === "dirty" ? "有未保存更改" : saveState === "saved" ? "Saved" : "未修改"}</div>
          {error && <div className="project-entry-error">{error}</div>}
        </div>
      </Modal>
    </>
  );
}
