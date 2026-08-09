use crate::constants::{
    APP_SETTINGS_RELATIVE_PATH, BUILTIN_GROK_BUILD_API_KEY_ENV,
    BUILTIN_GROK_BUILD_CREDENTIAL_ACCOUNT, CREDENTIAL_SERVICE, DECISION_API_KEY_ENV,
    DECISION_CREDENTIAL_ACCOUNT, DEEPSEEK_API_TIMEOUT_SECS, DEEPSEEK_API_URL,
    DEEPSEEK_WORKFLOW_MODEL, DEFAULT_BUILTIN_GROK_BUILD_API_BASE_URL,
    DEFAULT_BUILTIN_GROK_BUILD_MAX_TURNS, DEFAULT_BUILTIN_GROK_BUILD_MODEL,
    DEFAULT_VISION_MAX_IMAGES, DEFAULT_VISION_MAX_IMAGE_BYTES, DEFAULT_VISION_MAX_TOTAL_BYTES,
    DEFAULT_VISION_MODEL, DEFAULT_VISION_MODEL_REQUEST_URL, EXECUTION_ENGINE_TIMEOUT_SECS,
    LEGACY_BUILTIN_GROK_BUILD_API_KEY_ENV, LEGACY_DECISION_API_KEY_ENV, UPSTREAM_GROK_API_KEY_ENV,
    VISION_MODEL_CREDENTIAL_ACCOUNT,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const SETTINGS_SCHEMA_VERSION: u32 = 3;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VisionModelSettings {
    #[serde(default)]
    pub enabled: bool,
    pub request_url: String,
    pub model: String,
    pub timeout_secs: u64,
    pub max_image_bytes: u64,
    pub max_total_bytes: u64,
    pub max_images: u32,
}

impl Default for VisionModelSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            request_url: DEFAULT_VISION_MODEL_REQUEST_URL.to_string(),
            model: DEFAULT_VISION_MODEL.to_string(),
            timeout_secs: DEEPSEEK_API_TIMEOUT_SECS,
            max_image_bytes: DEFAULT_VISION_MAX_IMAGE_BYTES,
            max_total_bytes: DEFAULT_VISION_MAX_TOTAL_BYTES,
            max_images: DEFAULT_VISION_MAX_IMAGES,
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
    #[serde(default)]
    vision_model: VisionModelSettings,
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
    if raw.schema_version > SETTINGS_SCHEMA_VERSION {
        return Err(format!(
            "应用设置版本 {} 高于当前支持版本 {}，请升级 Metheus 后重试",
            raw.schema_version, SETTINGS_SCHEMA_VERSION
        ));
    }
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
        vision_model: raw.vision_model,
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
    #[serde(default)]
    pub vision_model: VisionModelSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            revision: default_settings_revision(),
            decision_model: DecisionModelSettings::default(),
            built_in_grok_build: BuiltInGrokBuildSettings::default(),
            plugin_cli: PluginCliSettings::default(),
            vision_model: VisionModelSettings::default(),
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
    pub vision_model: VisionModelSettings,
}

impl From<AppSettings> for AppSettingsInput {
    fn from(settings: AppSettings) -> Self {
        Self {
            decision_model: settings.decision_model,
            built_in_grok_build: settings.built_in_grok_build,
            plugin_cli: settings.plugin_cli,
            vision_model: settings.vision_model,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum SecretTarget {
    DecisionModel,
    BuiltInGrokBuild,
    VisionModel,
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
    pub vision_model_secret: SecretStatus,
    pub load_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum ModelConnectionTarget {
    DecisionModel,
    BuiltInGrokBuild,
    VisionModel,
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
    vision_model: Option<RuntimeSecret>,
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
    disk_fingerprint: Option<String>,
    write_blocked_reason: Option<String>,
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

fn release_activity(state: &mut RuntimeState, kind: ActivityKind) {
    match kind {
        ActivityKind::DecisionRequest => {
            state.active_decision_requests = state.active_decision_requests.saturating_sub(1)
        }
        ActivityKind::EngineOperation => {
            state.active_engine_operations = state.active_engine_operations.saturating_sub(1)
        }
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let Some(store) = SETTINGS_STORE.get() else {
            return;
        };
        let Ok(mut state) = store.state.lock() else {
            return;
        };
        release_activity(&mut state, self.kind);
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

pub(crate) struct VisionRequestSnapshot {
    pub settings_revision: u64,
    pub settings: VisionModelSettings,
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
    let (settings, load_warning, preserve_corrupt_file, disk_fingerprint, write_blocked_reason) =
        if !path.exists() {
            (AppSettings::default(), None, false, None, None)
        } else {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let fingerprint = Some(settings_content_fingerprint(&content));
                    match decode_settings(&content) {
                        Ok((settings, warning)) => (settings, warning, false, fingerprint, None),
                        Err(error) => {
                            let write_blocked_reason = future_schema_version(&content).map(|version| {
                            format!(
                                "磁盘设置版本 {version} 高于当前支持版本 {SETTINGS_SCHEMA_VERSION}，拒绝覆盖"
                            )
                        });
                            (
                                AppSettings::default(),
                                Some(format!(
                                    "应用设置文件无效，已使用默认设置；原文件保持不变：{error}"
                                )),
                                true,
                                fingerprint,
                                write_blocked_reason,
                            )
                        }
                    }
                }
                Err(error) => (
                    AppSettings::default(),
                    Some(format!(
                        "无法读取应用设置，已使用默认设置；原文件保持不变：{error}"
                    )),
                    true,
                    None,
                    Some("磁盘设置文件无法读取，拒绝覆盖".to_string()),
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
                vision_model: None,
            },
            load_warning,
            preserve_corrupt_file,
            disk_fingerprint,
            write_blocked_reason,
            active_decision_requests: 0,
            active_engine_operations: 0,
        }),
    }
}

fn settings_content_fingerprint(content: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(content.as_bytes()))
}

fn future_schema_version(content: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("schema_version")?
        .as_u64()
        .filter(|version| *version > SETTINGS_SCHEMA_VERSION as u64)
}

fn store() -> Result<&'static SettingsStore, String> {
    SETTINGS_STORE
        .get()
        .ok_or_else(|| "应用设置尚未初始化".to_string())
}

fn normalize_settings(mut settings: AppSettings) -> Result<AppSettings, String> {
    if settings.schema_version > SETTINGS_SCHEMA_VERSION {
        return Err(format!(
            "应用设置版本 {} 高于当前支持版本 {}，拒绝降级覆盖",
            settings.schema_version, SETTINGS_SCHEMA_VERSION
        ));
    }
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
    settings.vision_model.request_url = normalize_url(
        &settings.vision_model.request_url,
        "视觉模型请求地址",
        false,
    )?;
    validate_model(&settings.decision_model.model, "决策模型")?;
    validate_model(&settings.built_in_grok_build.model, "预装 Grok Build 模型")?;
    validate_model(&settings.vision_model.model, "视觉模型")?;
    validate_timeout(settings.decision_model.timeout_secs, "决策模型")?;
    validate_timeout(settings.built_in_grok_build.timeout_secs, "预装 Grok Build")?;
    validate_timeout(settings.vision_model.timeout_secs, "视觉模型")?;
    if !(1..=500).contains(&settings.built_in_grok_build.max_turns) {
        return Err("预装 Grok Build 最大执行轮数必须在 1 到 500 之间".to_string());
    }
    if settings.vision_model.max_image_bytes == 0
        || settings.vision_model.max_total_bytes < settings.vision_model.max_image_bytes
        || settings.vision_model.max_total_bytes > 100 * 1024 * 1024
    {
        return Err("视觉图片大小限制无效".to_string());
    }
    if !(1..=20).contains(&settings.vision_model.max_images) {
        return Err("视觉图片数量必须在 1 到 20 之间".to_string());
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
        SecretTarget::VisionModel => &[],
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
        SecretTarget::VisionModel => VISION_MODEL_CREDENTIAL_ACCOUNT,
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
    restore_credential_with_store(rollback, write_credential, delete_credential)
}

fn restore_credential_with_store<Write, Delete>(
    rollback: &SecretRollback,
    mut write: Write,
    mut delete: Delete,
) -> Result<(), String>
where
    Write: FnMut(&SecretTarget, &str) -> Result<(), String>,
    Delete: FnMut(&SecretTarget) -> Result<(), String>,
{
    match rollback.previous.as_deref() {
        Some(value) => write(&rollback.target, value),
        None => delete(&rollback.target),
    }
}

fn credential_error_after_rollback(error: String, rollbacks: &[Option<&SecretRollback>]) -> String {
    let rollback_errors = rollbacks
        .iter()
        .flatten()
        .filter_map(|rollback| restore_credential(rollback).err())
        .collect::<Vec<_>>();
    if rollback_errors.is_empty() {
        error
    } else {
        format!("{error}；恢复系统凭据失败：{}", rollback_errors.join("；"))
    }
}

fn secret_value(
    state: &RuntimeState,
    target: &SecretTarget,
) -> Result<Option<(String, SecretSource)>, String> {
    if *target == SecretTarget::VisionModel {
        return credential_value(target)
            .map(|value| value.map(|value| (value, SecretSource::SystemCredentialStore)));
    }
    let session = match target {
        SecretTarget::DecisionModel => state.secrets.decision_model.as_ref(),
        SecretTarget::BuiltInGrokBuild => state.secrets.built_in_grok_build.as_ref(),
        SecretTarget::VisionModel => state.secrets.vision_model.as_ref(),
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
    if target == SecretTarget::VisionModel {
        return match credential_value(&target) {
            Ok(Some(_)) => SecretStatus {
                configured: true,
                source: SecretSource::SystemCredentialStore,
                hint: "由系统凭据库提供".to_string(),
                persistent_available: true,
                persisted: true,
                persistence_error: None,
            },
            Ok(None) => SecretStatus {
                configured: false,
                source: SecretSource::Missing,
                hint: "未配置".to_string(),
                persistent_available: true,
                persisted: false,
                persistence_error: None,
            },
            Err(error) => SecretStatus {
                configured: false,
                source: SecretSource::Missing,
                hint: "系统凭据库不可用".to_string(),
                persistent_available: false,
                persisted: false,
                persistence_error: Some(error),
            },
        };
    }
    let session = match target {
        SecretTarget::DecisionModel => state.secrets.decision_model.as_ref(),
        SecretTarget::BuiltInGrokBuild => state.secrets.built_in_grok_build.as_ref(),
        SecretTarget::VisionModel => state.secrets.vision_model.as_ref(),
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
        vision_model_secret: secret_status(state, SecretTarget::VisionModel),
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
    apply_secret_mutation_with_store(
        target,
        current,
        mutation,
        credential_value,
        write_credential,
        delete_credential,
    )
}

fn apply_secret_mutation_with_store<Read, Write, Delete>(
    target: SecretTarget,
    current: &mut Option<RuntimeSecret>,
    mutation: SecretMutation,
    mut read: Read,
    mut write: Write,
    mut delete: Delete,
) -> Result<(bool, Option<SecretRollback>), String>
where
    Read: FnMut(&SecretTarget) -> Result<Option<String>, String>,
    Write: FnMut(&SecretTarget, &str) -> Result<(), String>,
    Delete: FnMut(&SecretTarget) -> Result<(), String>,
{
    match mutation {
        SecretMutation::Unchanged => Ok((false, None)),
        SecretMutation::Replace { value, persistence } => {
            if target == SecretTarget::VisionModel && persistence == SecretPersistence::SessionOnly
            {
                return Err("视觉模型 API Key 只允许保存到系统凭据库，不支持仅会话使用".to_string());
            }
            let value = validate_secret(value)?;
            let (source, rollback) = match persistence {
                SecretPersistence::SecureStore => {
                    let previous = read(&target)?;
                    let current_matches = current.as_ref().is_some_and(|secret| {
                        secret.value == value
                            && secret.source == SecretSource::SystemCredentialStore
                    });
                    if previous.as_deref() == Some(value.as_str()) && current_matches {
                        return Ok((false, None));
                    }
                    let changed_credential = previous.as_deref() != Some(value.as_str());
                    if changed_credential {
                        write(&target, &value)?;
                    }
                    (
                        SecretSource::SystemCredentialStore,
                        changed_credential.then(|| SecretRollback {
                            target: target.clone(),
                            previous,
                        }),
                    )
                }
                SecretPersistence::SessionOnly => {
                    let previous = read(&target)?;
                    let current_matches = current.as_ref().is_some_and(|secret| {
                        secret.value == value && secret.source == SecretSource::Session
                    });
                    if previous.is_none() && current_matches {
                        return Ok((false, None));
                    }
                    if previous.is_some() {
                        delete(&target)?;
                    }
                    (
                        SecretSource::Session,
                        previous.map(|previous| SecretRollback {
                            target: target.clone(),
                            previous: Some(previous),
                        }),
                    )
                }
            };
            *current = Some(RuntimeSecret { value, source });
            Ok((true, rollback))
        }
        SecretMutation::Clear => {
            let previous = read(&target)?;
            delete(&target)?;
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
    vision_model_secret: SecretMutation,
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
        vision_model: input.vision_model,
    };
    next_settings = normalize_settings(next_settings)?;
    let settings_changed = next_settings.decision_model != state.settings.decision_model
        || next_settings.built_in_grok_build != state.settings.built_in_grok_build
        || next_settings.plugin_cli != state.settings.plugin_cli;
    let settings_changed =
        settings_changed || next_settings.vision_model != state.settings.vision_model;

    let _disk_lock = acquire_settings_write_lock(&store.path)?;
    verify_settings_disk_snapshot(
        &store.path,
        state.settings.revision,
        state.disk_fingerprint.as_deref(),
        state.preserve_corrupt_file,
    )?;

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
            return Err(credential_error_after_rollback(
                error,
                &[decision_rollback.as_ref()],
            ));
        }
    };
    let (vision_changed, vision_rollback) = match apply_secret_mutation(
        SecretTarget::VisionModel,
        &mut next_secrets.vision_model,
        vision_model_secret,
    ) {
        Ok(result) => result,
        Err(error) => {
            return Err(credential_error_after_rollback(
                error,
                &[grok_rollback.as_ref(), decision_rollback.as_ref()],
            ));
        }
    };
    if !settings_changed && !decision_changed && !grok_changed && !vision_changed {
        return Ok(view_from_state(&state));
    }

    next_settings.revision = state.settings.revision.saturating_add(1);
    let persisted_fingerprint = match persist_settings_if_snapshot(
        &store.path,
        &next_settings,
        state.preserve_corrupt_file,
        state.settings.revision,
        state.disk_fingerprint.as_deref(),
    ) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return Err(credential_error_after_rollback(
                error,
                &[
                    vision_rollback.as_ref(),
                    grok_rollback.as_ref(),
                    decision_rollback.as_ref(),
                ],
            ));
        }
    };
    state.settings = next_settings;
    state.secrets = next_secrets;
    state.load_warning = None;
    state.preserve_corrupt_file = false;
    state.disk_fingerprint = Some(persisted_fingerprint);
    state.write_blocked_reason = None;
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
            SecretMutation::Unchanged,
        ),
        SecretTarget::VisionModel => update_settings(
            expected_revision,
            input,
            SecretMutation::Unchanged,
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
            SecretMutation::Unchanged,
        ),
        SecretTarget::BuiltInGrokBuild => update_settings(
            expected_revision,
            input,
            SecretMutation::Unchanged,
            SecretMutation::Clear,
            SecretMutation::Unchanged,
        ),
        SecretTarget::VisionModel => update_settings(
            expected_revision,
            input,
            SecretMutation::Unchanged,
            SecretMutation::Unchanged,
            SecretMutation::Clear,
        ),
    }
}

fn ensure_update_allowed(state: &RuntimeState, expected_revision: u64) -> Result<(), String> {
    if let Some(reason) = &state.write_blocked_reason {
        return Err(reason.clone());
    }
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

pub(crate) fn begin_vision_request() -> Result<VisionRequestSnapshot, String> {
    let activity = begin_activity(ActivityKind::DecisionRequest)?;
    (|| {
        let state = store()?
            .state
            .lock()
            .map_err(|_| "应用设置锁已损坏".to_string())?;
        if !state.settings.vision_model.enabled {
            return Err("视觉模型辅助未启用".to_string());
        }
        let api_key = secret_value(&state, &SecretTarget::VisionModel)?
            .map(|(value, _)| value)
            .ok_or_else(|| "视觉模型 API Key 未配置；请在应用设置中安全保存".to_string())?;
        Ok(VisionRequestSnapshot {
            settings_revision: state.settings.revision,
            settings: state.settings.vision_model.clone(),
            api_key,
            _activity: activity,
        })
    })()
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

struct SettingsWriteLock {
    path: PathBuf,
}

impl Drop for SettingsWriteLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn settings_write_lock_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "app-settings.json".into());
    name.push(".lock");
    path.with_file_name(name)
}

fn acquire_settings_write_lock(path: &Path) -> Result<SettingsWriteLock, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "应用设置路径缺少父目录".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建设置目录失败：{error}"))?;
    let lock_path = settings_write_lock_path(path);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|error| format!("另一个进程正在保存应用设置，当前写入已取消：{error}"))?;
    Ok(SettingsWriteLock { path: lock_path })
}

fn verify_settings_disk_snapshot(
    path: &Path,
    expected_revision: u64,
    expected_fingerprint: Option<&str>,
    allow_known_corrupt_file: bool,
) -> Result<(), String> {
    let content = match (expected_fingerprint, path.exists()) {
        (None, false) => return Ok(()),
        (None, true) => {
            return Err("应用设置磁盘冲突：加载后出现了新的设置文件，请同步后重试".to_string())
        }
        (Some(_), false) => {
            return Err("应用设置磁盘冲突：已加载的设置文件被外部删除，请同步后重试".to_string())
        }
        (Some(_), true) => std::fs::read_to_string(path)
            .map_err(|error| format!("复核应用设置磁盘状态失败：{error}"))?,
    };
    let actual_fingerprint = settings_content_fingerprint(&content);
    if expected_fingerprint != Some(actual_fingerprint.as_str()) {
        return Err("应用设置磁盘冲突：文件已被其他进程修改，请同步后重试".to_string());
    }
    if let Some(version) = future_schema_version(&content) {
        return Err(format!(
            "磁盘设置版本 {version} 高于当前支持版本 {SETTINGS_SCHEMA_VERSION}，拒绝覆盖"
        ));
    }
    match decode_settings(&content) {
        Ok((settings, _)) if settings.revision == expected_revision => Ok(()),
        Ok((settings, _)) => Err(format!(
            "应用设置磁盘修订冲突：加载修订 {expected_revision}，磁盘修订 {}",
            settings.revision
        )),
        Err(_) if allow_known_corrupt_file => Ok(()),
        Err(error) => Err(format!("应用设置磁盘内容无法安全复核：{error}")),
    }
}

fn persist_settings_if_snapshot(
    path: &Path,
    settings: &AppSettings,
    preserve_corrupt_file: bool,
    expected_revision: u64,
    expected_fingerprint: Option<&str>,
) -> Result<String, String> {
    verify_settings_disk_snapshot(
        path,
        expected_revision,
        expected_fingerprint,
        preserve_corrupt_file,
    )?;
    persist_settings(path, settings, preserve_corrupt_file)?;
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("复核已保存应用设置失败：{error}"))?;
    Ok(settings_content_fingerprint(&content))
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
    fn v2_settings_upgrade_to_v3_without_losing_existing_models_or_plugin_paths(
    ) -> Result<(), String> {
        let v2 = serde_json::json!({
            "schema_version": 2,
            "revision": 11,
            "decision_model": {
                "request_url": "https://decision.example/v1/chat/completions",
                "model": "decision-custom",
                "timeout_secs": 77,
                "structured_output": "PromptOnly"
            },
            "built_in_grok_build": {
                "api_backend": "Messages",
                "api_base_url": "https://grok.example/v1",
                "model": "grok-custom",
                "timeout_secs": 88,
                "max_turns": 19
            },
            "plugin_cli": {
                "claude_code_path": "/tools/claude",
                "codex_path": "/tools/codex",
                "kimi_path": "/tools/kimi",
                "grok_path": "/tools/grok"
            }
        });

        let (settings, _) = decode_settings(&v2.to_string())?;

        assert_eq!(settings.schema_version, 3);
        assert_eq!(settings.revision, 11);
        assert_eq!(settings.decision_model.model, "decision-custom");
        assert_eq!(settings.decision_model.timeout_secs, 77);
        assert_eq!(settings.built_in_grok_build.model, "grok-custom");
        assert_eq!(settings.built_in_grok_build.max_turns, 19);
        assert_eq!(
            settings.built_in_grok_build.api_backend,
            GrokBuildApiBackend::Messages
        );
        assert_eq!(
            settings.plugin_cli.claude_code_path.as_deref(),
            Some("/tools/claude")
        );
        assert_eq!(
            settings.plugin_cli.codex_path.as_deref(),
            Some("/tools/codex")
        );
        assert_eq!(
            settings.plugin_cli.kimi_path.as_deref(),
            Some("/tools/kimi")
        );
        assert_eq!(
            settings.plugin_cli.grok_path.as_deref(),
            Some("/tools/grok")
        );
        assert!(!settings.vision_model.enabled);
        Ok(())
    }

    #[test]
    fn v3_settings_preserve_known_vision_fields() -> Result<(), String> {
        let v3 = serde_json::json!({
            "schema_version": 3,
            "revision": 12,
            "vision_model": {
                "enabled": true,
                "request_url": "https://vision.example/v1/chat/completions",
                "model": "vision-custom",
                "timeout_secs": 91,
                "max_image_bytes": 1048576,
                "max_total_bytes": 4194304,
                "max_images": 4
            }
        });

        let (settings, _) = decode_settings(&v3.to_string())?;
        assert_eq!(settings.schema_version, 3);
        assert_eq!(settings.revision, 12);
        assert!(settings.vision_model.enabled);
        assert_eq!(settings.vision_model.model, "vision-custom");
        assert_eq!(settings.vision_model.max_images, 4);
        Ok(())
    }

    #[test]
    fn future_schema_is_rejected_and_disk_file_stays_byte_identical() -> Result<(), String> {
        let directory = TestDirectory::new("future-schema")?;
        let path = directory.path.join("app-settings.json");
        let original = r#"{"schema_version":4,"revision":99,"future":{"keep":true}}"#;
        std::fs::write(&path, original).map_err(|error| error.to_string())?;

        let store = load_store(path.clone());
        let state = store
            .state
            .lock()
            .map_err(|_| "测试设置锁已损坏".to_string())?;
        assert!(state.preserve_corrupt_file);
        assert!(state
            .load_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("高于当前支持版本")));
        assert!(state.write_blocked_reason.is_some());
        assert!(ensure_update_allowed(&state, state.settings.revision)
            .unwrap_err()
            .contains("拒绝覆盖"));
        drop(state);
        assert_eq!(
            std::fs::read_to_string(path).map_err(|error| error.to_string())?,
            original
        );
        Ok(())
    }

    #[test]
    fn disk_snapshot_cas_rejects_external_revision_without_overwrite() -> Result<(), String> {
        let directory = TestDirectory::new("disk-cas")?;
        let path = directory.path.join("app-settings.json");
        let mut initial = AppSettings::default();
        initial.revision = 7;
        initial.decision_model.model = "initial-model".to_string();
        persist_settings(&path, &initial, false)?;
        let loaded_content = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let loaded_fingerprint = settings_content_fingerprint(&loaded_content);

        let mut external = initial.clone();
        external.revision = 8;
        external.decision_model.model = "external-winner".to_string();
        persist_settings(&path, &external, false)?;

        let mut attempted = initial;
        attempted.revision = 8;
        attempted.decision_model.model = "stale-local".to_string();
        let _lock = acquire_settings_write_lock(&path)?;
        let error =
            persist_settings_if_snapshot(&path, &attempted, false, 7, Some(&loaded_fingerprint))
                .unwrap_err();
        assert!(error.contains("磁盘冲突"));
        let disk = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let (decoded, _) = decode_settings(&disk)?;
        assert_eq!(decoded.revision, 8);
        assert_eq!(decoded.decision_model.model, "external-winner");
        assert!(!disk.contains("stale-local"));
        Ok(())
    }

    #[test]
    fn vision_secret_policy_rejects_session_and_environment_sources_without_store_access() {
        assert!(secret_from_environment(&SecretTarget::VisionModel).is_none());
        let mut current = Some(RuntimeSecret {
            value: "legacy-session-secret".to_string(),
            source: SecretSource::Session,
        });
        let original = current.clone();
        let error = apply_secret_mutation_with_store(
            SecretTarget::VisionModel,
            &mut current,
            SecretMutation::Replace {
                value: "new-vision-secret".to_string(),
                persistence: SecretPersistence::SessionOnly,
            },
            |_| panic!("SessionOnly 拒绝不得读取凭据库"),
            |_, _| panic!("SessionOnly 拒绝不得写入凭据库"),
            |_| panic!("SessionOnly 拒绝不得删除凭据库"),
        )
        .unwrap_err();
        assert!(error.contains("只允许保存到系统凭据库"));
        assert_eq!(
            current.as_ref().map(|secret| &secret.value),
            original.as_ref().map(|secret| &secret.value)
        );
        assert_eq!(
            current.as_ref().map(|secret| &secret.source),
            original.as_ref().map(|secret| &secret.source)
        );
    }

    #[test]
    fn persistence_change_deletes_old_credential_and_delete_failure_keeps_memory(
    ) -> Result<(), String> {
        use std::cell::RefCell;

        let credential = RefCell::new(Some("persisted-old".to_string()));
        let mut current = Some(RuntimeSecret {
            value: "persisted-old".to_string(),
            source: SecretSource::SystemCredentialStore,
        });
        let (changed, rollback) = apply_secret_mutation_with_store(
            SecretTarget::DecisionModel,
            &mut current,
            SecretMutation::Replace {
                value: "session-new".to_string(),
                persistence: SecretPersistence::SessionOnly,
            },
            |_| Ok(credential.borrow().clone()),
            |_, value| {
                *credential.borrow_mut() = Some(value.to_string());
                Ok(())
            },
            |_| {
                credential.borrow_mut().take();
                Ok(())
            },
        )?;
        assert!(changed);
        assert!(credential.borrow().is_none());
        assert_eq!(
            current.as_ref().map(|secret| &secret.source),
            Some(&SecretSource::Session)
        );
        assert_eq!(
            rollback.and_then(|value| value.previous),
            Some("persisted-old".to_string())
        );

        let mut unchanged = Some(RuntimeSecret {
            value: "persisted-current".to_string(),
            source: SecretSource::SystemCredentialStore,
        });
        let error = apply_secret_mutation_with_store(
            SecretTarget::DecisionModel,
            &mut unchanged,
            SecretMutation::Replace {
                value: "session-new".to_string(),
                persistence: SecretPersistence::SessionOnly,
            },
            |_| Ok(Some("persisted-current".to_string())),
            |_, _| Ok(()),
            |_| Err("isolated delete failure".to_string()),
        )
        .unwrap_err();
        assert_eq!(error, "isolated delete failure");
        assert_eq!(
            unchanged
                .as_ref()
                .map(|secret| (secret.value.as_str(), &secret.source)),
            Some(("persisted-current", &SecretSource::SystemCredentialStore))
        );
        Ok(())
    }

    #[test]
    fn credential_rollback_restores_previous_or_removes_new_value() -> Result<(), String> {
        use std::cell::RefCell;

        let stored = RefCell::new(Some("new-value".to_string()));
        restore_credential_with_store(
            &SecretRollback {
                target: SecretTarget::VisionModel,
                previous: Some("previous-value".to_string()),
            },
            |_, value| {
                *stored.borrow_mut() = Some(value.to_string());
                Ok(())
            },
            |_| {
                stored.borrow_mut().take();
                Ok(())
            },
        )?;
        assert_eq!(stored.borrow().as_deref(), Some("previous-value"));

        restore_credential_with_store(
            &SecretRollback {
                target: SecretTarget::VisionModel,
                previous: None,
            },
            |_, value| {
                *stored.borrow_mut() = Some(value.to_string());
                Ok(())
            },
            |_| {
                stored.borrow_mut().take();
                Ok(())
            },
        )?;
        assert!(stored.borrow().is_none());
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

    fn runtime_state_for_activity_tests() -> RuntimeState {
        RuntimeState {
            settings: AppSettings::default(),
            secrets: RuntimeSecrets {
                decision_model: None,
                built_in_grok_build: None,
                vision_model: None,
            },
            load_warning: None,
            preserve_corrupt_file: false,
            disk_fingerprint: None,
            write_blocked_reason: None,
            active_decision_requests: 0,
            active_engine_operations: 0,
        }
    }

    #[test]
    fn phase1_runtime_contract_settings_updates_require_current_revision_and_no_active_leases() {
        let mut state = runtime_state_for_activity_tests();
        let current_revision = state.settings.revision;

        assert!(ensure_update_allowed(&state, current_revision).is_ok());
        assert!(ensure_update_allowed(&state, current_revision.saturating_add(1)).is_err());

        state.active_decision_requests = 1;
        assert!(ensure_update_allowed(&state, current_revision).is_err());
        state.active_decision_requests = 0;

        state.active_engine_operations = 1;
        assert!(ensure_update_allowed(&state, current_revision).is_err());
        state.active_engine_operations = 0;
        assert!(ensure_update_allowed(&state, current_revision).is_ok());
    }

    #[test]
    fn phase1_runtime_contract_activity_release_is_independent_saturating_and_unblocks_updates() {
        let mut state = runtime_state_for_activity_tests();
        let current_revision = state.settings.revision;
        state.active_decision_requests = 2;
        state.active_engine_operations = 2;

        release_activity(&mut state, ActivityKind::DecisionRequest);
        assert_eq!(state.active_decision_requests, 1);
        assert_eq!(state.active_engine_operations, 2);
        assert!(ensure_update_allowed(&state, current_revision).is_err());

        release_activity(&mut state, ActivityKind::DecisionRequest);
        release_activity(&mut state, ActivityKind::EngineOperation);
        assert_eq!(state.active_decision_requests, 0);
        assert_eq!(state.active_engine_operations, 1);
        assert!(ensure_update_allowed(&state, current_revision).is_err());

        release_activity(&mut state, ActivityKind::EngineOperation);
        assert!(ensure_update_allowed(&state, current_revision).is_ok());

        release_activity(&mut state, ActivityKind::DecisionRequest);
        release_activity(&mut state, ActivityKind::EngineOperation);
        assert_eq!(state.active_decision_requests, 0);
        assert_eq!(state.active_engine_operations, 0);
    }
}
