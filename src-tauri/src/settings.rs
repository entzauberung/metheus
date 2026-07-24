use crate::constants::{
    APP_SETTINGS_RELATIVE_PATH, BUILTIN_GROK_BUILD_API_KEY_ENV,
    BUILTIN_GROK_BUILD_CREDENTIAL_ACCOUNT, CREDENTIAL_SERVICE, DECISION_API_KEY_ENV,
    DECISION_CREDENTIAL_ACCOUNT, DEEPSEEK_API_TIMEOUT_SECS, DEEPSEEK_API_URL,
    DEEPSEEK_WORKFLOW_MODEL, DEFAULT_BUILTIN_GROK_BUILD_API_BASE_URL,
    DEFAULT_BUILTIN_GROK_BUILD_MAX_TURNS, DEFAULT_BUILTIN_GROK_BUILD_MODEL,
    EXECUTION_ENGINE_TIMEOUT_SECS, LEGACY_BUILTIN_GROK_BUILD_API_KEY_ENV,
    LEGACY_DECISION_API_KEY_ENV, UPSTREAM_GROK_API_KEY_ENV,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const SETTINGS_SCHEMA_VERSION: u32 = 2;
const MIN_TIMEOUT_SECS: u64 = 5;
const MAX_TIMEOUT_SECS: u64 = 3_600;
const MAX_MODEL_CHARS: usize = 200;
const MAX_SECRET_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum ApiInterface {
    #[default]
    OpenAiCompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum GrokBuildApiBackend {
    #[default]
    ChatCompletions,
    Responses,
    Messages,
}

impl GrokBuildApiBackend {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::ChatCompletions => "ChatCompletions",
            Self::Responses => "Responses",
            Self::Messages => "Messages",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum StructuredOutputPolicy {
    #[default]
    NativeJsonObject,
    PromptOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DecisionModelSettings {
    #[serde(default)]
    pub api_interface: ApiInterface,
    pub request_url: String,
    pub model: String,
    pub timeout_secs: u64,
    #[serde(default)]
    pub structured_output: StructuredOutputPolicy,
}

impl Default for DecisionModelSettings {
    fn default() -> Self {
        Self {
            api_interface: ApiInterface::OpenAiCompatible,
            request_url: DEEPSEEK_API_URL.to_string(),
            model: DEEPSEEK_WORKFLOW_MODEL.to_string(),
            timeout_secs: DEEPSEEK_API_TIMEOUT_SECS,
            structured_output: StructuredOutputPolicy::NativeJsonObject,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BuiltInGrokBuildSettings {
    #[serde(default)]
    pub api_backend: GrokBuildApiBackend,
    pub api_base_url: String,
    pub model: String,
    pub timeout_secs: u64,
    pub max_turns: u32,
}

impl Default for BuiltInGrokBuildSettings {
    fn default() -> Self {
        Self {
            api_backend: GrokBuildApiBackend::ChatCompletions,
            api_base_url: DEFAULT_BUILTIN_GROK_BUILD_API_BASE_URL.to_string(),
            model: DEFAULT_BUILTIN_GROK_BUILD_MODEL.to_string(),
            timeout_secs: EXECUTION_ENGINE_TIMEOUT_SECS,
            max_turns: DEFAULT_BUILTIN_GROK_BUILD_MAX_TURNS,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawAppSettings {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default = "default_settings_revision")]
    revision: u64,
    #[serde(default)]
    decision_model: DecisionModelSettings,
    #[serde(default)]
    built_in_grok_build: Option<BuiltInGrokBuildSettings>,
    #[serde(default)]
    built_in_grok: Option<LegacyBuiltInGrokSettings>,
    #[serde(default)]
    plugin_cli: PluginCliSettings,
}

#[derive(Debug, Deserialize)]
struct LegacyBuiltInGrokSettings {
    #[serde(default)]
    api_interface: ApiInterface,
    api_base_url: String,
    model: String,
    timeout_secs: u64,
    max_turns: u32,
}

impl From<LegacyBuiltInGrokSettings> for BuiltInGrokBuildSettings {
    fn from(legacy: LegacyBuiltInGrokSettings) -> Self {
        let _ = legacy.api_interface;
        Self {
            api_backend: GrokBuildApiBackend::ChatCompletions,
            api_base_url: legacy.api_base_url,
            model: legacy.model,
            timeout_secs: legacy.timeout_secs,
            max_turns: legacy.max_turns,
        }
    }
}

fn decode_settings(content: &str) -> Result<(AppSettings, Option<String>), String> {
    let raw: RawAppSettings =
        serde_json::from_str(content).map_err(|error| format!("解析失败：{error}"))?;
    let (built_in_grok_build, warning) = match (raw.built_in_grok_build, raw.built_in_grok) {
        (Some(current), Some(_)) => (
            current,
            Some("检测到新旧 Grok Build 设置字段；已采用 built_in_grok_build".to_string()),
        ),
        (Some(current), None) => (current, None),
        (None, Some(legacy)) => (
            legacy.into(),
            Some("已将旧 built_in_grok 设置迁移为 built_in_grok_build".to_string()),
        ),
        (None, None) => (BuiltInGrokBuildSettings::default(), None),
    };
    let settings = normalize_settings(AppSettings {
        schema_version: raw.schema_version,
        revision: raw.revision,
        decision_model: raw.decision_model,
        built_in_grok_build,
        plugin_cli: raw.plugin_cli,
    })?;
    Ok((settings, warning))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct PluginCliSettings {
    #[serde(default)]
    pub claude_code_path: Option<String>,
    #[serde(default)]
    pub codex_path: Option<String>,
    #[serde(default)]
    pub kimi_path: Option<String>,
    #[serde(default)]
    pub grok_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppSettings {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_settings_revision")]
    pub revision: u64,
    #[serde(default)]
    pub decision_model: DecisionModelSettings,
    #[serde(default)]
    pub built_in_grok_build: BuiltInGrokBuildSettings,
    #[serde(default)]
    pub plugin_cli: PluginCliSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            revision: default_settings_revision(),
            decision_model: DecisionModelSettings::default(),
            built_in_grok_build: BuiltInGrokBuildSettings::default(),
            plugin_cli: PluginCliSettings::default(),
        }
    }
}

fn default_schema_version() -> u32 {
    SETTINGS_SCHEMA_VERSION
}

fn default_settings_revision() -> u64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AppSettingsInput {
    pub decision_model: DecisionModelSettings,
    pub built_in_grok_build: BuiltInGrokBuildSettings,
    pub plugin_cli: PluginCliSettings,
}

impl From<AppSettings> for AppSettingsInput {
    fn from(settings: AppSettings) -> Self {
        Self {
            decision_model: settings.decision_model,
            built_in_grok_build: settings.built_in_grok_build,
            plugin_cli: settings.plugin_cli,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum SecretTarget {
    DecisionModel,
    BuiltInGrokBuild,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(tag = "action")]
pub(crate) enum SecretMutation {
    #[default]
    Unchanged,
    Replace {
        value: String,
        #[serde(default)]
        persistence: SecretPersistence,
    },
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum SecretPersistence {
    #[default]
    SecureStore,
    SessionOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum SecretSource {
    Session,
    SystemCredentialStore,
    Environment,
    LegacyEnvironment,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SecretStatus {
    pub configured: bool,
    pub source: SecretSource,
    pub hint: String,
    pub persistent_available: bool,
    pub persisted: bool,
    pub persistence_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppSettingsView {
    pub settings: AppSettings,
    pub decision_secret: SecretStatus,
    pub built_in_grok_build_secret: SecretStatus,
    pub load_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum ModelConnectionTarget {
    DecisionModel,
    BuiltInGrokBuild,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum ModelConnectionErrorKind {
    MissingSecret,
    InvalidConfiguration,
    Authentication,
    QuotaExceeded,
    RateLimited,
    Timeout,
    Network,
    ProviderUnavailable,
    Protocol,
    HttpStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConnectionTestResult {
    pub success: bool,
    pub target: ModelConnectionTarget,
    pub model: String,
    pub latency_ms: u64,
    pub error_kind: Option<ModelConnectionErrorKind>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct RuntimeSecrets {
    decision_model: Option<RuntimeSecret>,
    built_in_grok_build: Option<RuntimeSecret>,
}

#[derive(Debug, Clone)]
struct RuntimeSecret {
    value: String,
    source: SecretSource,
}

#[derive(Debug)]
struct SecretRollback {
    target: SecretTarget,
    previous: Option<String>,
}

#[derive(Debug)]
struct RuntimeState {
    settings: AppSettings,
    secrets: RuntimeSecrets,
    load_warning: Option<String>,
    preserve_corrupt_file: bool,
    active_decision_requests: usize,
    active_engine_operations: usize,
}

#[derive(Debug)]
struct SettingsStore {
    path: PathBuf,
    state: Mutex<RuntimeState>,
}

static SETTINGS_STORE: OnceLock<SettingsStore> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
enum ActivityKind {
    DecisionRequest,
    EngineOperation,
}

pub(crate) struct ActivityGuard {
    kind: ActivityKind,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let Some(store) = SETTINGS_STORE.get() else {
            return;
        };
        let Ok(mut state) = store.state.lock() else {
            return;
        };
        match self.kind {
            ActivityKind::DecisionRequest => {
                state.active_decision_requests = state.active_decision_requests.saturating_sub(1)
            }
            ActivityKind::EngineOperation => {
                state.active_engine_operations = state.active_engine_operations.saturating_sub(1)
            }
        }
    }
}

pub(crate) struct DecisionRequestSnapshot {
    pub settings_revision: u64,
    pub settings: DecisionModelSettings,
    pub api_key: String,
    pub _activity: ActivityGuard,
}

pub(crate) struct EngineOperationSnapshot {
    pub settings: AppSettings,
    pub built_in_grok_build_api_key: Option<String>,
    pub _activity: ActivityGuard,
}

pub(crate) struct BuiltInGrokBuildRequestSnapshot {
    pub settings_revision: u64,
    pub settings: BuiltInGrokBuildSettings,
    pub api_key: String,
    pub _activity: ActivityGuard,
}

pub(crate) fn settings_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户家目录路径".to_string())?;
    Ok(home.join(APP_SETTINGS_RELATIVE_PATH))
}

pub(crate) fn initialize_settings() -> Result<(), String> {
    if SETTINGS_STORE.get().is_some() {
        return Ok(());
    }
    let path = settings_path()?;
    let store = load_store(path);
    SETTINGS_STORE
        .set(store)
        .map_err(|_| "应用设置已被并发初始化".to_string())
}

fn load_store(path: PathBuf) -> SettingsStore {
    let (settings, load_warning, preserve_corrupt_file) = if !path.exists() {
        (AppSettings::default(), None, false)
    } else {
        match std::fs::read_to_string(&path) {
            Ok(content) => match decode_settings(&content) {
                Ok((settings, warning)) => (settings, warning, false),
                Err(error) => (
                    AppSettings::default(),
                    Some(format!(
                        "应用设置文件无效，已使用默认设置；原文件保持不变：{error}"
                    )),
                    true,
                ),
            },
            Err(error) => (
                AppSettings::default(),
                Some(format!(
                    "无法读取应用设置，已使用默认设置；原文件保持不变：{error}"
                )),
                true,
            ),
        }
    };

    SettingsStore {
        path,
        state: Mutex::new(RuntimeState {
            settings,
            secrets: RuntimeSecrets {
                decision_model: None,
                built_in_grok_build: None,
            },
            load_warning,
            preserve_corrupt_file,
            active_decision_requests: 0,
            active_engine_operations: 0,
        }),
    }
}

fn store() -> Result<&'static SettingsStore, String> {
    SETTINGS_STORE
        .get()
        .ok_or_else(|| "应用设置尚未初始化".to_string())
}

fn normalize_settings(mut settings: AppSettings) -> Result<AppSettings, String> {
    settings.schema_version = SETTINGS_SCHEMA_VERSION;
    if settings.revision == 0 {
        settings.revision = default_settings_revision();
    }
    settings.decision_model.request_url = normalize_url(
        &settings.decision_model.request_url,
        "决策模型请求地址",
        false,
    )?;
    settings.built_in_grok_build.api_base_url = normalize_url(
        &settings.built_in_grok_build.api_base_url,
        "预装 Grok Build 接口地址",
        true,
    )?;
    validate_model(&settings.decision_model.model, "决策模型")?;
    validate_model(&settings.built_in_grok_build.model, "预装 Grok Build 模型")?;
    validate_timeout(settings.decision_model.timeout_secs, "决策模型")?;
    validate_timeout(settings.built_in_grok_build.timeout_secs, "预装 Grok Build")?;
    if !(1..=500).contains(&settings.built_in_grok_build.max_turns) {
        return Err("预装 Grok Build 最大执行轮数必须在 1 到 500 之间".to_string());
    }
    normalize_plugin_paths(&mut settings.plugin_cli);
    Ok(settings)
}

fn normalize_url(value: &str, label: &str, trim_trailing_slash: bool) -> Result<String, String> {
    let parsed =
        reqwest::Url::parse(value.trim()).map_err(|error| format!("{label}无效：{error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("{label}只支持 http 或 https"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("{label}不得包含用户名或密码"));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(format!("{label}不得包含 query 或 fragment"));
    }
    let mut normalized = parsed.to_string();
    if trim_trailing_slash {
        normalized = normalized.trim_end_matches('/').to_string();
    }
    Ok(normalized)
}

fn validate_model(value: &str, label: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label}名称不能为空"));
    }
    if value.chars().count() > MAX_MODEL_CHARS {
        return Err(format!("{label}名称不能超过 {MAX_MODEL_CHARS} 个字符"));
    }
    Ok(())
}

fn validate_timeout(value: u64, label: &str) -> Result<(), String> {
    if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&value) {
        return Err(format!(
            "{label}超时必须在 {MIN_TIMEOUT_SECS} 到 {MAX_TIMEOUT_SECS} 秒之间"
        ));
    }
    Ok(())
}

fn normalize_plugin_paths(settings: &mut PluginCliSettings) {
    for path in [
        &mut settings.claude_code_path,
        &mut settings.codex_path,
        &mut settings.kimi_path,
        &mut settings.grok_path,
    ] {
        *path = path
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
}

fn validate_secret(value: String) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err("API Key 不能包含控制字符".to_string());
    }
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    if value.len() > MAX_SECRET_BYTES {
        return Err(format!("API Key 不能超过 {MAX_SECRET_BYTES} 字节"));
    }
    Ok(value)
}

fn secret_from_environment(target: &SecretTarget) -> Option<(String, SecretSource, &'static str)> {
    let variables: &[(&str, SecretSource)] = match target {
        SecretTarget::DecisionModel => &[
            (DECISION_API_KEY_ENV, SecretSource::Environment),
            (LEGACY_DECISION_API_KEY_ENV, SecretSource::LegacyEnvironment),
        ],
        SecretTarget::BuiltInGrokBuild => &[
            (BUILTIN_GROK_BUILD_API_KEY_ENV, SecretSource::Environment),
            (
                LEGACY_BUILTIN_GROK_BUILD_API_KEY_ENV,
                SecretSource::LegacyEnvironment,
            ),
            (UPSTREAM_GROK_API_KEY_ENV, SecretSource::LegacyEnvironment),
        ],
    };
    for (variable, source) in variables {
        let Ok(value) = std::env::var(variable) else {
            continue;
        };
        if !value.trim().is_empty() {
            return Some((value, source.clone(), variable));
        }
    }
    None
}

fn credential_account(target: &SecretTarget) -> &'static str {
    match target {
        SecretTarget::DecisionModel => DECISION_CREDENTIAL_ACCOUNT,
        SecretTarget::BuiltInGrokBuild => BUILTIN_GROK_BUILD_CREDENTIAL_ACCOUNT,
    }
}

fn credential_entry(target: &SecretTarget) -> Result<keyring::Entry, String> {
    keyring::Entry::new(CREDENTIAL_SERVICE, credential_account(target))
        .map_err(|error| format!("系统凭据库不可用：{error}"))
}

fn credential_value(target: &SecretTarget) -> Result<Option<String>, String> {
    match credential_entry(target)?.get_password() {
        Ok(value) => validate_secret(value).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("无法读取系统凭据：{error}")),
    }
}

fn write_credential(target: &SecretTarget, value: &str) -> Result<(), String> {
    credential_entry(target)?
        .set_password(value)
        .map_err(|error| format!("无法安全保存 API Key：{error}"))
}

fn delete_credential(target: &SecretTarget) -> Result<(), String> {
    match credential_entry(target)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("无法从系统凭据库清除 API Key：{error}")),
    }
}

fn restore_credential(rollback: &SecretRollback) -> Result<(), String> {
    match rollback.previous.as_deref() {
        Some(value) => write_credential(&rollback.target, value),
        None => delete_credential(&rollback.target),
    }
}

fn secret_value(
    state: &RuntimeState,
    target: &SecretTarget,
) -> Result<Option<(String, SecretSource)>, String> {
    let session = match target {
        SecretTarget::DecisionModel => state.secrets.decision_model.as_ref(),
        SecretTarget::BuiltInGrokBuild => state.secrets.built_in_grok_build.as_ref(),
    };
    if let Some(secret) = session {
        return Ok(Some((secret.value.clone(), secret.source.clone())));
    }
    if let Ok(Some(value)) = credential_value(target) {
        return Ok(Some((value, SecretSource::SystemCredentialStore)));
    }
    secret_from_environment(target)
        .map(|(value, source, variable)| {
            validate_secret(value)
                .map(|value| (value, source))
                .map_err(|error| format!("环境变量 {variable} 无效：{error}"))
        })
        .transpose()
}

fn secret_status(state: &RuntimeState, target: SecretTarget) -> SecretStatus {
    let session = match target {
        SecretTarget::DecisionModel => state.secrets.decision_model.as_ref(),
        SecretTarget::BuiltInGrokBuild => state.secrets.built_in_grok_build.as_ref(),
    };
    if let Some(secret) = session {
        let persisted = secret.source == SecretSource::SystemCredentialStore;
        return SecretStatus {
            configured: true,
            source: secret.source.clone(),
            hint: if persisted {
                "已安全保存到系统凭据库".to_string()
            } else {
                "仅本次会话使用".to_string()
            },
            persistent_available: true,
            persisted,
            persistence_error: None,
        };
    }
    let credential = credential_value(&target);
    if let Ok(Some(_)) = credential {
        return SecretStatus {
            configured: true,
            source: SecretSource::SystemCredentialStore,
            hint: "由系统凭据库提供".to_string(),
            persistent_available: true,
            persisted: true,
            persistence_error: None,
        };
    }
    let persistence_error = credential.err();
    if let Some((value, source, variable)) = secret_from_environment(&target) {
        if let Err(error) = validate_secret(value) {
            return SecretStatus {
                configured: false,
                source: SecretSource::Missing,
                hint: format!("环境变量 {variable} 无效"),
                persistent_available: persistence_error.is_none(),
                persisted: false,
                persistence_error: Some(error),
            };
        }
        return SecretStatus {
            configured: true,
            source,
            hint: format!("由 {variable} 环境变量提供"),
            persistent_available: persistence_error.is_none(),
            persisted: false,
            persistence_error,
        };
    }
    SecretStatus {
        configured: false,
        source: SecretSource::Missing,
        hint: "未配置".to_string(),
        persistent_available: persistence_error.is_none(),
        persisted: false,
        persistence_error,
    }
}

fn view_from_state(state: &RuntimeState) -> AppSettingsView {
    AppSettingsView {
        settings: state.settings.clone(),
        decision_secret: secret_status(state, SecretTarget::DecisionModel),
        built_in_grok_build_secret: secret_status(state, SecretTarget::BuiltInGrokBuild),
        load_warning: state.load_warning.clone(),
    }
}

pub(crate) fn app_settings_view() -> Result<AppSettingsView, String> {
    let state = store()?
        .state
        .lock()
        .map_err(|_| "应用设置锁已损坏".to_string())?;
    Ok(view_from_state(&state))
}

pub(crate) fn settings_snapshot() -> Result<AppSettings, String> {
    let state = store()?
        .state
        .lock()
        .map_err(|_| "应用设置锁已损坏".to_string())?;
    Ok(state.settings.clone())
}

pub(crate) fn endpoint_fingerprint(endpoint: &str) -> String {
    let digest = Sha256::digest(endpoint.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn apply_secret_mutation(
    target: SecretTarget,
    current: &mut Option<RuntimeSecret>,
    mutation: SecretMutation,
) -> Result<(bool, Option<SecretRollback>), String> {
    match mutation {
        SecretMutation::Unchanged => Ok((false, None)),
        SecretMutation::Replace { value, persistence } => {
            let value = validate_secret(value)?;
            if current.as_ref().is_some_and(|secret| {
                secret.value == value
                    && secret.source
                        == match persistence {
                            SecretPersistence::SecureStore => SecretSource::SystemCredentialStore,
                            SecretPersistence::SessionOnly => SecretSource::Session,
                        }
            }) {
                return Ok((false, None));
            }
            let (source, rollback) = match persistence {
                SecretPersistence::SecureStore => {
                    let previous = credential_value(&target)?;
                    write_credential(&target, &value)?;
                    (
                        SecretSource::SystemCredentialStore,
                        Some(SecretRollback { target, previous }),
                    )
                }
                SecretPersistence::SessionOnly => (SecretSource::Session, None),
            };
            *current = Some(RuntimeSecret { value, source });
            Ok((true, rollback))
        }
        SecretMutation::Clear => {
            let previous = credential_value(&target)?;
            delete_credential(&target)?;
            let changed = current.take().is_some() || previous.is_some();
            Ok((changed, Some(SecretRollback { target, previous })))
        }
    }
}

pub(crate) fn update_settings(
    expected_revision: u64,
    input: AppSettingsInput,
    decision_secret: SecretMutation,
    built_in_grok_build_secret: SecretMutation,
) -> Result<AppSettingsView, String> {
    let store = store()?;
    let mut state = store
        .state
        .lock()
        .map_err(|_| "应用设置锁已损坏".to_string())?;
    ensure_update_allowed(&state, expected_revision)?;

    let mut next_settings = AppSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        revision: state.settings.revision,
        decision_model: input.decision_model,
        built_in_grok_build: input.built_in_grok_build,
        plugin_cli: input.plugin_cli,
    };
    next_settings = normalize_settings(next_settings)?;
    let settings_changed = next_settings.decision_model != state.settings.decision_model
        || next_settings.built_in_grok_build != state.settings.built_in_grok_build
        || next_settings.plugin_cli != state.settings.plugin_cli;

    let mut next_secrets = state.secrets.clone();
    let (decision_changed, decision_rollback) = apply_secret_mutation(
        SecretTarget::DecisionModel,
        &mut next_secrets.decision_model,
        decision_secret,
    )?;
    let (grok_changed, grok_rollback) = match apply_secret_mutation(
        SecretTarget::BuiltInGrokBuild,
        &mut next_secrets.built_in_grok_build,
        built_in_grok_build_secret,
    ) {
        Ok(result) => result,
        Err(error) => {
            if let Some(rollback) = decision_rollback.as_ref() {
                let _ = restore_credential(rollback);
            }
            return Err(error);
        }
    };
    if !settings_changed && !decision_changed && !grok_changed {
        return Ok(view_from_state(&state));
    }

    next_settings.revision = state.settings.revision.saturating_add(1);
    if let Err(error) = persist_settings(&store.path, &next_settings, state.preserve_corrupt_file) {
        let mut rollback_errors = Vec::new();
        for rollback in [grok_rollback.as_ref(), decision_rollback.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Err(rollback_error) = restore_credential(rollback) {
                rollback_errors.push(rollback_error);
            }
        }
        return if rollback_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}；恢复系统凭据失败：{}",
                rollback_errors.join("；")
            ))
        };
    }
    state.settings = next_settings;
    state.secrets = next_secrets;
    state.load_warning = None;
    state.preserve_corrupt_file = false;
    Ok(view_from_state(&state))
}

pub(crate) fn replace_secret(
    expected_revision: u64,
    target: SecretTarget,
    secret: String,
    persistence: SecretPersistence,
) -> Result<AppSettingsView, String> {
    let current = settings_snapshot()?;
    let input = AppSettingsInput::from(current);
    match target {
        SecretTarget::DecisionModel => update_settings(
            expected_revision,
            input,
            SecretMutation::Replace {
                value: secret,
                persistence,
            },
            SecretMutation::Unchanged,
        ),
        SecretTarget::BuiltInGrokBuild => update_settings(
            expected_revision,
            input,
            SecretMutation::Unchanged,
            SecretMutation::Replace {
                value: secret,
                persistence,
            },
        ),
    }
}

pub(crate) fn clear_secret(
    expected_revision: u64,
    target: SecretTarget,
) -> Result<AppSettingsView, String> {
    let current = settings_snapshot()?;
    let input = AppSettingsInput::from(current);
    match target {
        SecretTarget::DecisionModel => update_settings(
            expected_revision,
            input,
            SecretMutation::Clear,
            SecretMutation::Unchanged,
        ),
        SecretTarget::BuiltInGrokBuild => update_settings(
            expected_revision,
            input,
            SecretMutation::Unchanged,
            SecretMutation::Clear,
        ),
    }
}

fn ensure_update_allowed(state: &RuntimeState, expected_revision: u64) -> Result<(), String> {
    if state.settings.revision != expected_revision {
        return Err(format!(
            "应用设置已更新，请同步后重试（当前修订 {}，请求修订 {}）",
            state.settings.revision, expected_revision
        ));
    }
    if state.active_decision_requests > 0 || state.active_engine_operations > 0 {
        return Err("AI 请求或执行任务正在进行，暂时不能修改应用设置".to_string());
    }
    Ok(())
}

fn begin_activity(kind: ActivityKind) -> Result<ActivityGuard, String> {
    let store = store()?;
    let mut state = store
        .state
        .lock()
        .map_err(|_| "应用设置锁已损坏".to_string())?;
    match kind {
        ActivityKind::DecisionRequest => {
            state.active_decision_requests = state.active_decision_requests.saturating_add(1)
        }
        ActivityKind::EngineOperation => {
            state.active_engine_operations = state.active_engine_operations.saturating_add(1)
        }
    }
    Ok(ActivityGuard { kind })
}

pub(crate) fn begin_decision_request() -> Result<DecisionRequestSnapshot, String> {
    let activity = begin_activity(ActivityKind::DecisionRequest)?;
    let result = (|| {
        let state = store()?
            .state
            .lock()
            .map_err(|_| "应用设置锁已损坏".to_string())?;
        let api_key = secret_value(&state, &SecretTarget::DecisionModel)?
            .map(|(value, _)| value)
            .ok_or_else(|| {
                format!(
                    "决策模型 API Key 未配置；请在应用设置中填写，或设置 {DECISION_API_KEY_ENV}"
                )
            })?;
        Ok(DecisionRequestSnapshot {
            settings_revision: state.settings.revision,
            settings: state.settings.decision_model.clone(),
            api_key,
            _activity: activity,
        })
    })();
    result
}

pub(crate) fn begin_engine_operation() -> Result<EngineOperationSnapshot, String> {
    let activity = begin_activity(ActivityKind::EngineOperation)?;
    let result = (|| {
        let state = store()?
            .state
            .lock()
            .map_err(|_| "应用设置锁已损坏".to_string())?;
        Ok(EngineOperationSnapshot {
            settings: state.settings.clone(),
            built_in_grok_build_api_key: secret_value(&state, &SecretTarget::BuiltInGrokBuild)?
                .map(|(value, _)| value),
            _activity: activity,
        })
    })();
    result
}

pub(crate) fn begin_built_in_grok_build_request() -> Result<BuiltInGrokBuildRequestSnapshot, String>
{
    let activity = begin_activity(ActivityKind::DecisionRequest)?;
    let result = (|| {
        let state = store()?
            .state
            .lock()
            .map_err(|_| "应用设置锁已损坏".to_string())?;
        let api_key = secret_value(&state, &SecretTarget::BuiltInGrokBuild)?
            .map(|(value, _)| value)
            .ok_or_else(|| {
                format!(
                    "预装 Grok Build API Key 未配置；请在应用设置中填写，或设置 {BUILTIN_GROK_BUILD_API_KEY_ENV}"
                )
            })?;
        Ok(BuiltInGrokBuildRequestSnapshot {
            settings_revision: state.settings.revision,
            settings: state.settings.built_in_grok_build.clone(),
            api_key,
            _activity: activity,
        })
    })();
    result
}

fn persist_settings(
    path: &Path,
    settings: &AppSettings,
    preserve_corrupt_file: bool,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "应用设置路径缺少父目录".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建设置目录失败：{error}"))?;
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("序列化应用设置失败：{error}"))?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&temporary, json).map_err(|error| format!("写入设置临时文件失败：{error}"))?;

    let backup = if path.exists() && preserve_corrupt_file {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let backup = path.with_file_name(format!("app-settings.corrupt-{timestamp}.json"));
        std::fs::rename(path, &backup).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("备份损坏设置文件失败：{error}")
        })?;
        Some(backup)
    } else {
        None
    };

    if let Err(error) = replace_file(&temporary, path) {
        if let Some(backup) = backup {
            let _ = std::fs::rename(backup, path);
        }
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(source, destination).map_err(|error| format!("替换应用设置失败：{error}"))
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    let backup = destination.with_extension(format!("json.bak-{}", std::process::id()));
    let had_destination = destination.exists();
    if had_destination {
        std::fs::rename(destination, &backup)
            .map_err(|error| format!("准备替换应用设置失败：{error}"))?;
    }
    if let Err(error) = std::fs::rename(source, destination) {
        if had_destination {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(format!("替换应用设置失败：{error}"));
    }
    if had_destination {
        let _ = std::fs::remove_file(backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Result<Self, String> {
            let path = std::env::temp_dir()
                .join(format!("metheus-settings-{label}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            Ok(Self { path })
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn defaults_preserve_the_existing_decision_model() {
        let settings = AppSettings::default();
        assert_eq!(settings.decision_model.request_url, DEEPSEEK_API_URL);
        assert_eq!(settings.decision_model.model, DEEPSEEK_WORKFLOW_MODEL);
        assert_eq!(settings.revision, 1);
    }

    #[test]
    fn serialized_settings_never_contain_runtime_secrets() -> Result<(), String> {
        let settings = AppSettings::default();
        let value = serde_json::to_string(&settings).map_err(|error| error.to_string())?;
        assert!(!value.contains("metheus-secret-sentinel"));
        assert!(!value.contains("api_key"));
        assert!(value.contains("built_in_grok_build"));
        assert!(!value.contains("\"built_in_grok\":"));
        Ok(())
    }

    #[test]
    fn legacy_built_in_grok_settings_migrate_without_losing_values() -> Result<(), String> {
        let legacy = serde_json::json!({
            "schema_version": 1,
            "revision": 7,
            "decision_model": DecisionModelSettings::default(),
            "built_in_grok": {
                "api_interface": "OpenAiCompatible",
                "api_base_url": "https://example.test/v1/",
                "model": "legacy-model",
                "timeout_secs": 42,
                "max_turns": 9
            },
            "plugin_cli": PluginCliSettings::default()
        });
        let (settings, warning) = decode_settings(&legacy.to_string())?;
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.revision, 7);
        assert_eq!(settings.built_in_grok_build.model, "legacy-model");
        assert_eq!(settings.built_in_grok_build.timeout_secs, 42);
        assert_eq!(settings.built_in_grok_build.max_turns, 9);
        assert_eq!(
            settings.built_in_grok_build.api_backend,
            GrokBuildApiBackend::ChatCompletions
        );
        assert!(warning.is_some());
        Ok(())
    }

    #[test]
    fn current_built_in_grok_build_field_wins_over_legacy_field() -> Result<(), String> {
        let value = serde_json::json!({
            "built_in_grok_build": {
                "api_backend": "Responses",
                "api_base_url": "https://current.test/v1",
                "model": "current-model",
                "timeout_secs": 60,
                "max_turns": 10
            },
            "built_in_grok": {
                "api_interface": "OpenAiCompatible",
                "api_base_url": "https://legacy.test/v1",
                "model": "legacy-model",
                "timeout_secs": 30,
                "max_turns": 5
            }
        });
        let (settings, warning) = decode_settings(&value.to_string())?;
        assert_eq!(settings.built_in_grok_build.model, "current-model");
        assert_eq!(
            settings.built_in_grok_build.api_backend,
            GrokBuildApiBackend::Responses
        );
        assert!(warning.is_some());
        Ok(())
    }

    #[test]
    fn invalid_settings_are_preserved_until_an_explicit_save() -> Result<(), String> {
        let directory = TestDirectory::new("corrupt")?;
        let path = directory.path.join("app-settings.json");
        std::fs::write(&path, "{not-json").map_err(|error| error.to_string())?;
        let store = load_store(path.clone());
        let state = store
            .state
            .lock()
            .map_err(|_| "测试设置锁已损坏".to_string())?;
        assert!(state.load_warning.is_some());
        assert!(state.preserve_corrupt_file);
        drop(state);
        assert_eq!(
            std::fs::read_to_string(path).map_err(|error| error.to_string())?,
            "{not-json"
        );
        Ok(())
    }

    #[test]
    fn url_validation_rejects_embedded_credentials_and_queries() {
        assert!(normalize_url("https://user:secret@example.com/v1", "地址", false).is_err());
        assert!(normalize_url("https://example.com/v1?key=secret", "地址", false).is_err());
        assert!(normalize_url("http://localhost:8080/v1/chat/completions", "地址", false).is_ok());
    }

    #[test]
    fn secret_validation_rejects_control_characters_before_trimming() {
        assert!(validate_secret("valid-secret".to_string()).is_ok());
        assert!(validate_secret("secret\n".to_string()).is_err());
        assert!(validate_secret("secret\0tail".to_string()).is_err());
    }

    #[test]
    fn non_sensitive_settings_round_trip_without_secrets() -> Result<(), String> {
        let directory = TestDirectory::new("round-trip")?;
        let path = directory.path.join("app-settings.json");
        let mut settings = AppSettings::default();
        settings.revision = 9;
        settings.decision_model.model = "custom-decision-model".to_string();
        settings.plugin_cli.kimi_path = Some("/opt/metheus/kimi".to_string());
        persist_settings(&path, &settings, false)?;

        let store = load_store(path.clone());
        let state = store
            .state
            .lock()
            .map_err(|_| "测试设置锁已损坏".to_string())?;
        assert_eq!(state.settings, settings);
        assert!(state.secrets.decision_model.is_none());
        assert!(state.secrets.built_in_grok_build.is_none());
        drop(state);
        let serialized = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        assert!(!serialized.contains("api_key"));
        Ok(())
    }

    #[test]
    fn stale_revision_and_active_engine_lease_block_updates() {
        let mut state = RuntimeState {
            settings: AppSettings::default(),
            secrets: RuntimeSecrets {
                decision_model: None,
                built_in_grok_build: None,
            },
            load_warning: None,
            preserve_corrupt_file: false,
            active_decision_requests: 0,
            active_engine_operations: 0,
        };
        assert!(ensure_update_allowed(&state, state.settings.revision.saturating_add(1)).is_err());
        state.active_engine_operations = 1;
        assert!(ensure_update_allowed(&state, state.settings.revision).is_err());
        state.active_engine_operations = 0;
        assert!(ensure_update_allowed(&state, state.settings.revision).is_ok());
    }
}
