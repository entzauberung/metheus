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
  EngineFailureKind,
  EngineHealth,
  EngineRuntimeConfigurationEvidence,
  EngineRuntimeSelfTestResult,
  ExecutionProfile,
  ExecutionProvider,
  ModelConnectionTarget,
  ModelConnectionErrorKind,
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
import { presentPluginHealth } from "../engineHealthPresentation";
import { IconButton } from "./IconButton";
import { Modal } from "./Modal";
import { ModelServiceNavigation, type ModelServiceTarget } from "./ModelServiceNavigation";

type SettingsTab = "engine" | "automation" | "models";
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
];

const PLUGIN_PROVIDERS: ExecutionProvider[] = ["ClaudeCode", "Codex", "KimiCli", "GrokBuild"];
const PLUGIN_LABELS: Record<ExecutionProvider, string> = {
  ClaudeCode: "Claude Code",
  Codex: "Codex",
  KimiCli: "Kimi CLI",
  GrokBuild: "Grok Build CLI（本机）",
};

const ENGINE_FAILURE_LABELS: Record<EngineFailureKind, string> = {
  QuotaExceeded: "额度不足",
  AuthenticationError: "认证失败",
  RateLimited: "请求被限流",
  ProviderUnavailable: "服务暂不可用",
  NetworkError: "网络错误",
  Timeout: "验证超时",
  ProcessCrash: "CLI 进程异常",
  ToolRejected: "工具权限被拒绝",
  ProtocolError: "执行协议错误",
  OutputTruncated: "模型输出截断",
  MaxTurnsExceeded: "执行轮数已耗尽",
  RuntimeError: "执行运行时错误",
  TaskExecutionError: "验证请求失败",
};

const CONNECTION_FAILURE_LABELS: Record<ModelConnectionErrorKind, string> = {
  MissingSecret: "未配置凭据",
  InvalidConfiguration: "配置无效",
  Authentication: "认证失败",
  QuotaExceeded: "额度不足",
  RateLimited: "请求被限流",
  Timeout: "连接超时",
  Network: "网络错误",
  ProviderUnavailable: "服务暂不可用",
  Protocol: "响应协议错误",
  HttpStatus: "服务返回错误状态",
};

const SAFE_CONFIGURATION_VALUE = /^[A-Za-z0-9._/:+-]{1,96}$/;

function configurationFact(
  evidence: EngineRuntimeConfigurationEvidence | undefined,
  field: "model" | "reasoning_effort",
): string {
  const value = evidence?.[field];
  const source = evidence?.[`${field}_source`];
  return source === "Confirmed" && value && SAFE_CONFIGURATION_VALUE.test(value)
    ? value
    : "CLI 默认（未公开）";
}

function pluginFailureReason(health?: EngineHealth): string {
  const kind = health?.authentication?.failure_kind;
  return kind ? ENGINE_FAILURE_LABELS[kind] : "在线验证未通过";
}

function connectionResultSummary(result: ConnectionTestResult): string {
  if (result.success) return result.message;
  return `连接测试失败：${result.error_kind ? CONNECTION_FAILURE_LABELS[result.error_kind] : "原因未公开"}`;
}

function runtimeResultSummary(result: EngineRuntimeSelfTestResult): string {
  return result.success ? "运行时自检通过" : "运行时自检失败";
}

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
  const [modelTarget, setModelTarget] = useState<ModelServiceTarget>("decision");
  const [modelNavigationFocusRequest, setModelNavigationFocusRequest] = useState(0);
  const settingsTabRefs = useRef<Array<HTMLButtonElement | null>>([]);
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
  const [closeConfirmation, setCloseConfirmation] = useState(false);
  const requestId = useRef(0);
  const pendingSettingsWrite = useRef<Promise<unknown> | null>(null);
  const backgroundWriteError = useRef("");
  const blockedReason = writeBlockedReason || changeBlockedReason(project, pipeline);

  const focusSettingsTab = (index: number) => {
    settingsTabRefs.current[index]?.focus();
  };

  const activateSettingsTab = (nextTab: SettingsTab, requestModelFocus: boolean) => {
    setTab(nextTab);
    setConnectionResult(null);
    setRuntimeResult(null);
    setModelNavigationFocusRequest(requestModelFocus && nextTab === "models" ? (current) => current + 1 : 0);
  };

  const ownsRequest = (owner: number) => requestId.current === owner;

  const trackSettingsWrite = <T,>(operation: Promise<T>): Promise<T> => {
    backgroundWriteError.current = "";
    let tracked: Promise<T>;
    tracked = operation
      .catch((writeError) => {
        backgroundWriteError.current = String(writeError);
        throw writeError;
      })
      .finally(() => {
        if (pendingSettingsWrite.current === tracked) pendingSettingsWrite.current = null;
      });
    pendingSettingsWrite.current = tracked;
    return tracked;
  };

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

  const finishClose = () => {
    requestId.current += 1;
    resetSecrets();
    setView(null);
    setDraft(null);
    setLoading(false);
    setSaving(false);
    setTesting(null);
    setRuntimeTesting(false);
    setPluginChecking(false);
    setVerifyingProvider(null);
    setConnectionResult(null);
    setRuntimeResult(null);
    setCloseConfirmation(false);
    setError("");
    setOpen(false);
  };

  const close = () => {
    if (saving) {
      finishClose();
      return;
    }
    if (dirty) {
      setCloseConfirmation(true);
      setError("仍有未保存的设置草稿；请选择保存并关闭、放弃更改或继续编辑。");
      if (saveState !== "error") setSaveState("dirty");
      return;
    }
    finishClose();
  };

  const applyView = (next: AppSettingsView) => {
    setView(next);
    setDraft(structuredClone(next.settings));
    resetSecrets();
    setAutoSaveBlocked(false);
    setCloseConfirmation(false);
  };

  const markDirty = () => setSaveState((current) => current === "error" ? "error" : "dirty");

  useEffect(() => {
    const openDecisionSettings = () => {
      setModelTarget("decision");
      setTab("models");
      setModelNavigationFocusRequest((current) => current + 1);
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
    setPluginHealth({});
    const load = async () => {
      try {
        await pendingSettingsWrite.current?.catch(() => undefined);
        if (!ownsRequest(currentRequest)) return;
        const next = await invokeWithTimeout<AppSettingsView>("get_app_settings");
        if (!ownsRequest(currentRequest)) return;
        applyView(next);
        if (backgroundWriteError.current) {
          setError(`上次后台保存未完成：${backgroundWriteError.current}`);
          backgroundWriteError.current = "";
        }
      } catch (loadError) {
        if (ownsRequest(currentRequest)) setError(String(loadError));
      } finally {
        if (ownsRequest(currentRequest)) setLoading(false);
      }
    };
    void load();
  }, [open]);

  useEffect(() => {
    if (!open || tab !== "engine" || !view) return;
    const owner = requestId.current;
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
        if (active && ownsRequest(owner)) setPluginHealth(Object.fromEntries(entries));
      })
      .catch((healthError) => {
        if (active && ownsRequest(owner)) setError(String(healthError));
      })
      .finally(() => {
        if (active && ownsRequest(owner)) setPluginChecking(false);
      });
    return () => {
      active = false;
      if (ownsRequest(owner)) setPluginChecking(false);
    };
  }, [open, tab, view?.settings.revision]);

  const persistDraft = (): Promise<AppSettingsView> => {
    if (!view || !draft) return Promise.reject(new Error("应用设置尚未加载"));
    const operation = invokeWithTimeout<AppSettingsView>("update_app_settings", {
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
    return trackSettingsWrite(operation);
  };

  const discardAndClose = () => finishClose();

  const saveAndClose = () => {
    if (!dirty || blockedReason) return;
    setSaving(true);
    setSaveState("saving");
    const operation = persistDraft();
    finishClose();
    void operation.catch(() => undefined);
  };

  useEffect(() => {
    if (!open || !view || !draft || !nonSensitiveDirty || blockedReason || saving || autoSaveBlocked) return;
    const owner = requestId.current;
    const snapshot = structuredClone(draft);
    const snapshotKey = JSON.stringify(snapshot);
    const expectedRevision = view.settings.revision;
    const timer = window.setTimeout(() => {
      setSaving(true);
      setSaveState("saving");
      setError("");
      trackSettingsWrite(invokeWithTimeout<AppSettingsView>("update_app_settings", {
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
      }))
        .then((next) => {
          if (!ownsRequest(owner)) return;
          setView(next);
          setDraft((current) => (
            current && JSON.stringify(current) === snapshotKey
              ? structuredClone(next.settings)
              : current
          ));
          setAutoSaveBlocked(false);
          setSaveState("saved");
          setCloseConfirmation(false);
        })
        .catch(async (saveError) => {
          const message = String(saveError);
          if (!ownsRequest(owner)) return;
          backgroundWriteError.current = "";
          setAutoSaveBlocked(true);
          setSaveState("error");
          setError(message);
          if (message.includes("修订") || message.includes("磁盘冲突")) {
            try {
              const latest = await invokeWithTimeout<AppSettingsView>("get_app_settings");
              if (!ownsRequest(owner)) return;
              setView(latest);
              setError(`${message}；已同步最新服务端基线，本地草稿仍保留，请确认后重试保存。`);
            } catch {
              // The original conflict remains the actionable error.
            }
          }
        })
        .finally(() => {
          if (ownsRequest(owner)) setSaving(false);
        });
    }, 700);
    return () => window.clearTimeout(timer);
  }, [open, view, draft, nonSensitiveDirty, blockedReason, saving, autoSaveBlocked]);

  const handleSave = async () => {
    if (!dirty || blockedReason) return;
    const owner = requestId.current;
    setSaving(true);
    setAutoSaveBlocked(true);
    setSaveState("saving");
    setError("");
    setConnectionResult(null);
    try {
      const next = await persistDraft();
      if (!ownsRequest(owner)) return;
      applyView(next);
      setAutoSaveBlocked(false);
      setSaveState("saved");
    } catch (saveError) {
      const message = String(saveError);
      if (!ownsRequest(owner)) return;
      backgroundWriteError.current = "";
      setAutoSaveBlocked(true);
      setSaveState("error");
      setError(message);
      if (message.includes("修订") || message.includes("磁盘冲突")) {
        try {
          const latest = await invokeWithTimeout<AppSettingsView>("get_app_settings");
          if (!ownsRequest(owner)) return;
          setView(latest);
          setError(`${message}；已同步最新服务端基线，本地草稿仍保留，请确认后重试保存。`);
        } catch {
          // Keep the original conflict and local draft.
        }
      }
    } finally {
      if (ownsRequest(owner)) setSaving(false);
    }
  };

  const handleTest = async (target: ModelConnectionTarget) => {
    if (!draft || blockedReason) return;
    const owner = requestId.current;
    setTesting(target);
    setError("");
    setConnectionResult(null);
    try {
      const saved = dirty ? await persistDraft() : view;
      if (!ownsRequest(owner)) return;
      if (!saved) throw new Error("应用设置尚未加载");
      if (dirty) applyView(saved);
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
      if (!ownsRequest(owner)) return;
      setConnectionResult(result);
    } catch {
      if (ownsRequest(owner)) {
        backgroundWriteError.current = "";
        setError("模型连接测试请求失败，请重试。");
      }
    } finally {
      if (ownsRequest(owner)) setTesting(null);
    }
  };

  const handleRuntimeTest = async () => {
    if (!draft || blockedReason) return;
    const owner = requestId.current;
    setRuntimeTesting(true);
    setError("");
    setRuntimeResult(null);
    try {
      const saved = dirty ? await persistDraft() : view;
      if (!ownsRequest(owner)) return;
      if (!saved) throw new Error("应用设置尚未加载");
      if (dirty) applyView(saved);
      const result = await invokeWithTimeout<EngineRuntimeSelfTestResult>(
        "test_grok_build_runtime",
        undefined,
        (saved.settings.built_in_grok_build.timeout_secs + 10) * 1000,
      );
      if (!ownsRequest(owner)) return;
      setRuntimeResult(result);
      invalidateEngineHealth(BUILT_IN_GROK_BUILD_HEALTH_TARGET);
    } catch {
      if (ownsRequest(owner)) {
        backgroundWriteError.current = "";
        setError("运行时自检请求失败，请重试。");
      }
    } finally {
      if (ownsRequest(owner)) setRuntimeTesting(false);
    }
  };

  const handleVerifyPlugin = async (provider: ExecutionProvider) => {
    if (!draft || blockedReason) return;
    const owner = requestId.current;
    setVerifyingProvider(provider);
    setError("");
    try {
      if (dirty) {
        const saved = await persistDraft();
        if (!ownsRequest(owner)) return;
        applyView(saved);
      }
      let verificationError: unknown = null;
      try {
        await invokeWithTimeout<EngineAuthenticationResult>(
          "verify_engine_authentication",
          { executionProfile: pluginProfile(provider) },
        );
        invalidateEngineHealth({ runtime: "Plugin", provider });
      } catch (verifyError) {
        verificationError = verifyError;
      }
      if (!ownsRequest(owner)) return;
      const health = await invokeWithTimeout<EngineHealth>(
        "check_engine_health",
        { executionProfile: pluginProfile(provider) },
      );
      if (!ownsRequest(owner)) return;
      setPluginHealth((current) => ({ ...current, [provider]: health }));
      if (verificationError) setError("在线验证未完成，请查看该插件的最终状态和诊断详情后重试。");
    } catch (verifyError) {
      if (ownsRequest(owner)) {
        backgroundWriteError.current = "";
        setError("在线验证请求失败，请重试。");
      }
    } finally {
      if (ownsRequest(owner)) setVerifyingProvider(null);
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
    const owner = requestId.current;
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
      if (ownsRequest(owner)) setSaveState("saved");
    } catch (policyError) {
      if (ownsRequest(owner)) {
        setSaveState("error");
        setError(String(policyError));
      } else {
        backgroundWriteError.current = String(policyError);
      }
    } finally {
      if (ownsRequest(owner)) setSaving(false);
    }
  };

  const commitSecret = async (
    target: "DecisionModel" | "BuiltInGrokBuild" | "VisionModel",
    value: string,
    persistence: SecretPersistence,
  ) => {
    if (!view || !value.trim() || blockedReason || busy) return;
    const owner = requestId.current;
    setSaving(true);
    setSaveState("saving");
    setError("");
    try {
      const operation = (async () => {
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
        return invokeWithTimeout<AppSettingsView>("set_api_secret", {
          expectedRevision: current.settings.revision,
          target,
          secret: value,
          persistence,
        });
      })();
      const next = await trackSettingsWrite(operation);
      if (!ownsRequest(owner)) return;
      applyView(next);
      setSaveState("saved");
    } catch (secretError) {
      if (ownsRequest(owner)) {
        backgroundWriteError.current = "";
        setSaveState("error");
        setError(String(secretError));
      }
    } finally {
      if (ownsRequest(owner)) setSaving(false);
    }
  };

  const clearSecret = async (
    target: "DecisionModel" | "BuiltInGrokBuild" | "VisionModel",
  ) => {
    if (!view || blockedReason || busy) return;
    const owner = requestId.current;
    setSaving(true);
    setSaveState("saving");
    setError("");
    try {
      const next = await trackSettingsWrite(invokeWithTimeout<AppSettingsView>("clear_api_secret", {
        expectedRevision: view.settings.revision,
        target,
      }));
      if (!ownsRequest(owner)) return;
      applyView(next);
      setSaveState("saved");
    } catch (secretError) {
      if (ownsRequest(owner)) {
        backgroundWriteError.current = "";
        setSaveState("error");
        setError(String(secretError));
      }
    } finally {
      if (ownsRequest(owner)) setSaving(false);
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
  const handleModelTargetChange = (target: ModelServiceTarget) => {
    setModelTarget(target);
  };
  const modalActions = closeConfirmation
    ? [
        { label: "继续编辑", onClick: () => setCloseConfirmation(false), variant: "secondary" as const },
        { label: "放弃更改", onClick: discardAndClose, variant: "danger" as const },
        { label: "保存并关闭", onClick: saveAndClose, variant: "primary" as const, disabled: Boolean(blockedReason) },
      ]
    : [
        { label: "关闭", onClick: close, variant: "secondary" as const },
        { label: saving ? "保存中..." : saveState === "error" ? "重试保存" : "保存", onClick: handleSave, variant: "primary" as const, disabled: !dirty || busy || Boolean(blockedReason) },
      ];

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
        isSubmitting={saving}
        actions={modalActions}
      >
        <div className="application-settings">
          {closeConfirmation && (
            <div className="settings-close-confirmation" role="alertdialog" aria-labelledby="settings-close-title">
              <strong id="settings-close-title">有未保存的设置</strong>
              <p>保存并关闭会继续已提交的原子写入；放弃更改只丢弃当前本地草稿。</p>
            </div>
          )}
          <div className="settings-tabs" role="tablist" aria-label="应用设置分类">
            {TABS.map((item, index) => (
              <button
                ref={(button) => {
                  settingsTabRefs.current[index] = button;
                }}
                id={`settings-tab-${item.id}`}
                type="button"
                role="tab"
                aria-selected={tab === item.id}
                aria-controls={`settings-panel-${item.id}`}
                tabIndex={tab === item.id ? 0 : -1}
                className={tab === item.id ? "selected" : ""}
                key={item.id}
                onClick={() => activateSettingsTab(item.id, item.id === "models")}
                onKeyDown={(event) => {
                  let nextIndex = index;
                  if (event.key === "ArrowLeft") {
                    nextIndex = (index + TABS.length - 1) % TABS.length;
                  } else if (event.key === "ArrowRight") {
                    nextIndex = (index + 1) % TABS.length;
                  } else if (event.key === "Home") {
                    nextIndex = 0;
                  } else if (event.key === "End") {
                    nextIndex = TABS.length - 1;
                  } else {
                    return;
                  }
                  event.preventDefault();
                  activateSettingsTab(TABS[nextIndex].id, false);
                  focusSettingsTab(nextIndex);
                }}
              >
                {item.label}
              </button>
            ))}
          </div>

          {TABS.filter((item) => (
            item.id !== tab
            || (!loading && item.id === "engine" && !draft)
            || (!loading && item.id === "models" && (!draft || !view))
          )).map((item) => (
            <div
              id={`settings-panel-${item.id}`}
              role="tabpanel"
              aria-labelledby={`settings-tab-${item.id}`}
              hidden
              key={item.id}
            />
          ))}

          {loading && (
            <div
              className="settings-state"
              id={`settings-panel-${tab}`}
              role="tabpanel"
              aria-labelledby={`settings-tab-${tab}`}
            >
              正在读取设置...
            </div>
          )}
          {!loading && draft && view && tab === "models" && (
            <div className="settings-form" id="settings-panel-models" role="tabpanel" aria-labelledby="settings-tab-models">
              <ModelServiceNavigation
                value={modelTarget}
                onChange={handleModelTargetChange}
                focusRequest={modelNavigationFocusRequest}
              />
              {(["decision", "builtin-grok", "vision"] as const)
                .filter((target) => target !== modelTarget)
                .map((target) => (
                  <div
                    id={`model-service-panel-${target}`}
                    role="tabpanel"
                    aria-labelledby={`model-service-tab-${target}`}
                    hidden
                    key={target}
                  />
                ))}
              {modelTarget === "decision" && (
                <div className="settings-form" id="model-service-panel-decision" role="tabpanel" aria-labelledby="model-service-tab-decision">
                  <div className="settings-model-status-header" role="status" aria-live="polite"><span>当前状态</span><strong>{connectionResult?.target === "DecisionModel" ? (connectionResult.success ? "连接已验证" : "连接测试失败") : view.decision_secret.configured ? "已配置" : "未配置"}</strong></div>
                  <h3 id="decision-settings-title">决策模型</h3>
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
                  {connectionResult?.target === "DecisionModel" && (
                    <div className={`settings-connection settings-inline-result ${connectionResult.success ? "success" : "failure"}`} role="status">
                      {connectionResult.message}{connectionResult.success ? ` · ${connectionResult.latency_ms} ms` : ""}
                    </div>
                  )}
                </div>
              )}
              {modelTarget === "builtin-grok" && (
                <div className="settings-form" id="model-service-panel-builtin-grok" role="tabpanel" aria-labelledby="model-service-tab-builtin-grok">
                  <div className="settings-model-status-header" role="status" aria-live="polite"><span>当前状态</span><strong>{runtimeResult ? runtimeResultSummary(runtimeResult) : connectionResult?.target === "BuiltInGrokBuild" ? (connectionResult.success ? "连接已验证" : "连接测试失败") : view.built_in_grok_build_secret.configured ? "待运行时自检" : "未配置"}</strong></div>
                  <h3 id="grok-settings-title">内置 Grok Build</h3>
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
                  <p className="settings-hint">模型连接测试仅用于诊断接口与凭据；运行时自检是内置引擎的可执行门禁。</p>
                  {connectionResult?.target === "BuiltInGrokBuild" && (
                    <div className={`settings-connection settings-inline-result ${connectionResult.success ? "success" : "failure"}`} role="status">
                      {connectionResultSummary(connectionResult)}{connectionResult.success ? ` · ${connectionResult.latency_ms} ms` : ""}
                    </div>
                  )}
                  {runtimeResult && (
                    <div className={`settings-connection settings-inline-result ${runtimeResult.success ? "success" : "failure"}`} role="status">
                      {runtimeResultSummary(runtimeResult)} · 源码 {runtimeResult.source_revision.slice(0, 8)}
                    </div>
                  )}
                </div>
              )}
              {modelTarget === "vision" && (
                <div className="settings-form" id="model-service-panel-vision" role="tabpanel" aria-labelledby="model-service-tab-vision">
                  <div className="settings-model-status-header" role="status" aria-live="polite"><span>当前状态</span><strong>{!draft.vision_model.enabled ? "已关闭" : connectionResult?.target === "VisionModel" ? (connectionResult.success ? "连接已验证" : "连接测试失败") : "待测试"}</strong></div>
                  <h3 id="vision-settings-title">视觉模型（默认关闭）</h3>
                  <div className="settings-vision-toggle">
                    <input
                      aria-checked={draft.vision_model.enabled}
                      aria-describedby="vision-service-state vision-service-disabled-reason"
                      checked={draft.vision_model.enabled}
                      disabled={busy || Boolean(blockedReason)}
                      onChange={(event) => updateVision({ enabled: event.target.checked })}
                      role="switch"
                      type="checkbox"
                    />
                    <span>
                      <strong>启用视觉模型服务</strong>
                      <span id="vision-service-state">{draft.vision_model.enabled ? "视觉服务已启用" : "视觉服务已关闭"}</span>
                    </span>
                  </div>
                  {blockedReason && <p id="vision-service-disabled-reason" className="settings-hint">{blockedReason}</p>}
                  {!draft.vision_model.enabled ? (
                    <div className="settings-vision-status settings-disabled-state" role="status">
                      <strong>视觉模型服务保持关闭</strong>
                      <span>启用后才会显示连接、限制和系统凭据设置；不会自动采集或发送图片。</span>
                    </div>
                  ) : (
                    <div className="settings-vision-details">
                      <div className="settings-secret-row">
                        <label>API Key<span className="settings-secret-hint">{view.vision_model_secret.hint}</span><span className="settings-secret-input"><input ref={visionSecretRef} type={showVisionSecret ? "text" : "password"} autoComplete="off" disabled={busy || !view.vision_model_secret.persistent_available} placeholder={visionCredentialReady ? "保持不变" : "输入 API Key"} onBlur={(event) => commitSecret("VisionModel", event.currentTarget.value, "SecureStore")} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); commitSecret("VisionModel", event.currentTarget.value, "SecureStore"); } }} onChange={() => { setVisionSecretPending(Boolean(visionSecretRef.current?.value.trim())); markDirty(); }} /><button type="button" title={showVisionSecret ? "隐藏密钥" : "显示密钥"} disabled={busy} onClick={() => setShowVisionSecret((shown) => !shown)}>{showVisionSecret ? <EyeOff size={15} /> : <Eye size={15} />}</button></span></label>
                        <button type="button" className="settings-command" disabled={busy || !visionCredentialReady} onClick={() => clearSecret("VisionModel")}><Trash2 size={14} /> 清除</button>
                      </div>
                      <p className="settings-hint">密钥保存方式：仅系统凭据库（keyring）。不读取环境变量，也不支持仅会话保存。</p>
                      <details className="settings-vision-advanced" open>
                        <summary>连接与限制</summary>
                        <div className="settings-vision-advanced-body">
                          <label>请求地址<input type="url" value={draft.vision_model.request_url} disabled={busy} onChange={(event) => updateVision({ request_url: event.target.value })} /></label>
                          <label>模型名称<input value={draft.vision_model.model} disabled={busy} onChange={(event) => updateVision({ model: event.target.value })} /></label>
                          <div className="settings-grid-two">
                            <label>请求超时（秒）<input type="number" min={5} max={3600} step={1} value={draft.vision_model.timeout_secs} disabled={busy} onChange={(event) => updateVision({ timeout_secs: numberValue(event.target.value) })} /></label>
                            <label>最多图片数（张）<input type="number" min={1} max={20} step={1} value={draft.vision_model.max_images} disabled={busy} onChange={(event) => updateVision({ max_images: numberValue(event.target.value) })} /></label>
                            <label>单图上限（bytes）<input type="number" min={1} step={1} value={draft.vision_model.max_image_bytes} disabled={busy} onChange={(event) => updateVision({ max_image_bytes: numberValue(event.target.value) })} /></label>
                            <label>总大小上限（bytes）<input type="number" min={1} step={1} value={draft.vision_model.max_total_bytes} disabled={busy} onChange={(event) => updateVision({ max_total_bytes: numberValue(event.target.value) })} /></label>
                          </div>
                          <p className="settings-hint">数值按原始秒、数量和 bytes 无损保存，不会自动换算单位。</p>
                        </div>
                      </details>
                      <p className="settings-hint">视觉结果只作为人工确认的辅助证据，不会自动采用。</p>
                      {view.vision_model_secret.persistence_error && (
                        <p className="settings-warning" role="alert">{view.vision_model_secret.persistence_error}</p>
                      )}
                      {visionBlockedReason && <p className="settings-warning" role="alert">{visionBlockedReason}</p>}
                      <button type="button" className="settings-test" disabled={busy || Boolean(blockedReason) || Boolean(visionBlockedReason)} onClick={() => handleTest("VisionModel")}><FlaskConical size={15} /> {testing === "VisionModel" ? "测试中..." : "用微型 PNG 测试视觉能力"}</button>
                      {connectionResult?.target === "VisionModel" && (
                        <div className={`settings-connection settings-inline-result ${connectionResult.success ? "success" : "failure"}`} role="status">
                          {connectionResult.message}{connectionResult.success ? ` · ${connectionResult.latency_ms} ms` : ""}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
          {!loading && project && tab === "automation" && (
            <div className="settings-form" id="settings-panel-automation" role="tabpanel" aria-labelledby="settings-tab-automation">
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
            <div className="settings-state" id="settings-panel-automation" role="tabpanel" aria-labelledby="settings-tab-automation">打开项目后可配置人工确认和视觉辅助策略。</div>
          )}

          {!loading && draft && tab === "engine" && (
            <div className="settings-form" id="settings-panel-engine" role="tabpanel" aria-labelledby="settings-tab-engine">
              <h3>外部 CLI 插件</h3>
              <label>Claude Code 路径<input value={draft.plugin_cli.claude_code_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("claude_code_path", event.target.value)} /></label>
              <label>Codex 路径<input value={draft.plugin_cli.codex_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("codex_path", event.target.value)} /></label>
              <label>Kimi CLI 路径<input value={draft.plugin_cli.kimi_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("kimi_path", event.target.value)} /></label>
              <label>Grok Build CLI 路径<input value={draft.plugin_cli.grok_path ?? ""} disabled={busy} placeholder="从 PATH 自动查找" onChange={(event) => updatePlugin("grok_path", event.target.value)} /></label>
              <p className="settings-hint">外部 CLI 始终使用你的本机默认模型和 Provider；Metheus 不注入模型参数。</p>
              <div className="plugin-health-list" aria-live="polite">
                {PLUGIN_PROVIDERS.map((provider) => {
                  const health = pluginHealth[provider];
                  const presentation = presentPluginHealth(health);
                  const verificationSucceeded = health?.status === "Available";
                  const verificationFailed = health?.status === "VerificationFailed";
                  return (
                    <div className={`plugin-health-row tone-${presentation.tone}`} key={provider}>
                      <div className="plugin-health-name">
                        <span>{PLUGIN_LABELS[provider]}</span>
                        <span className="plugin-health-final">{presentation.label}</span>
                      </div>
                      {verificationSucceeded ? (
                        <dl className="plugin-verification-facts" aria-label={`${PLUGIN_LABELS[provider]} 在线验证配置事实`}>
                          <div><dt>当前模型</dt><dd>{configurationFact(health.authentication.runtime_configuration, "model")}</dd></div>
                          <div><dt>思考强度</dt><dd>{configurationFact(health.authentication.runtime_configuration, "reasoning_effort")}</dd></div>
                        </dl>
                      ) : verificationFailed ? (
                        <div className="plugin-verification-failure" role="status">
                          <span>原因</span>
                          <strong>{pluginFailureReason(health)}</strong>
                        </div>
                      ) : (
                        <dl className="plugin-health-facts">
                          <div><dt>安装</dt><dd>{health?.status === "NotInstalled" ? "未安装" : health ? "已发现" : "检查中"}</dd></div>
                          <div><dt>能力</dt><dd>{health?.status === "UnsupportedVersion" ? "不兼容" : health?.capabilities.length ? "已验证" : "未知"}</dd></div>
                          <div><dt>本地配置</dt><dd>{localAuthLabel(health?.authentication)}</dd></div>
                          <div><dt>在线认证</dt><dd>{onlineAuthLabel(health?.authentication)}</dd></div>
                        </dl>
                      )}
                      <div className="plugin-health-explanation">
                        <p>{presentation.summary}</p>
                        {!verificationSucceeded && <span>下一步：{presentation.actionLabel}</span>}
                        {!verificationSucceeded && !verificationFailed && (
                          <details>
                            <summary>诊断详情</summary>
                            <p>{presentation.detail}</p>
                          </details>
                        )}
                      </div>
                      <button
                        type="button"
                        className="settings-command plugin-verify"
                        disabled={busy || Boolean(blockedReason) || health?.status === "NotInstalled"}
                        onClick={() => handleVerifyPlugin(provider)}
                      >
                        <RefreshCw size={14} className={verifyingProvider === provider ? "engine-health-spinner" : ""} />
                        {verifyingProvider === provider ? "验证中..." : presentation.runnable ? "重新在线验证" : "在线验证"}
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {view?.load_warning && <div className="settings-warning">{view.load_warning}</div>}
          {blockedReason && <div className="settings-warning">{blockedReason}</div>}
          <div className={`settings-save-state ${saveState}`}>{saveState === "saving" ? "Saving…" : saveState === "error" ? "保存失败" : saveState === "dirty" ? "有未保存更改" : saveState === "saved" ? "Saved" : "未修改"}</div>
          {error && <div className="project-entry-error">{error}</div>}
        </div>
      </Modal>
    </>
  );
}
