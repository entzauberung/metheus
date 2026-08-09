use super::contract::{
    EngineAuthState, EngineAuthVerificationMethod, EngineAuthenticationResult, EngineHealth,
    EngineHealthStatus, EngineLocalAuthState, EngineOnlineAuthState, ProcessSpec, ProgramSource,
};
use crate::project::{EngineFailureKind, ExecutionProfile, ExecutionProvider, ExecutionRuntime};
use crate::settings::AppSettings;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

const CAPABILITY_PROBE_TIMEOUT_SECS: u64 = 5;
const ONLINE_AUTH_PROBE_TIMEOUT_SECS: u64 = 30;
const AUTH_RESULT_TTL_SECS: u64 = 5 * 60;
const AUTH_PROBE_CONTRACT_VERSION: &str = "minimal-ok-v2";
pub(super) const MINIMAL_PROBE_PROMPT: &str = "Reply with OK only. Do not use tools.";

#[derive(Clone)]
struct CachedAuthentication {
    result: EngineAuthenticationResult,
    expires_at: Instant,
}

static AUTH_CACHE: OnceLock<Mutex<HashMap<String, CachedAuthentication>>> = OnceLock::new();

struct AuthProbeDirectory(PathBuf);

impl AuthProbeDirectory {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "metheus-auth-probe-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("创建认证验证临时目录失败：{error}"))?;
        Ok(Self(path))
    }
}

impl Drop for AuthProbeDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct HealthCheckResult {
    pub health: EngineHealth,
    pub program: Option<OsString>,
    pub program_source: Option<ProgramSource>,
}

fn executable_configuration<'a>(
    provider: &ExecutionProvider,
    settings: &'a AppSettings,
) -> (Option<&'a str>, &'static [&'static str]) {
    match provider {
        ExecutionProvider::ClaudeCode => {
            (settings.plugin_cli.claude_code_path.as_deref(), &["claude"])
        }
        ExecutionProvider::Codex => (settings.plugin_cli.codex_path.as_deref(), &["codex"]),
        ExecutionProvider::KimiCli => (
            settings.plugin_cli.kimi_path.as_deref(),
            super::kimi_cli::EXECUTABLE_CANDIDATES,
        ),
        ExecutionProvider::GrokBuild => (
            settings.plugin_cli.grok_path.as_deref(),
            super::grok_cli::EXECUTABLE_CANDIDATES,
        ),
    }
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(windows)]
fn path_candidates(directory: &Path, name: &str) -> Vec<PathBuf> {
    let extensions = std::env::var_os("PATHEXT")
        .map(|value| {
            value
                .to_string_lossy()
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| vec![".EXE".to_string(), ".CMD".to_string(), ".BAT".to_string()]);
    let mut candidates = vec![directory.join(name)];
    candidates.extend(
        extensions
            .into_iter()
            .map(|extension| directory.join(format!("{name}{extension}"))),
    );
    candidates
}

#[cfg(not(windows))]
fn path_candidates(directory: &Path, name: &str) -> Vec<PathBuf> {
    vec![directory.join(name)]
}

fn find_executable(candidates: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in candidates {
            for candidate in path_candidates(&directory, name) {
                if is_executable_file(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn resolve_executable(
    configured: Option<&str>,
    candidates: &[&str],
) -> Result<(PathBuf, ProgramSource), String> {
    if let Some(configured) = configured {
        let path = PathBuf::from(configured);
        if !path.is_absolute() {
            return Err("可执行文件覆盖路径必须是绝对路径".to_string());
        }
        if !is_executable_file(&path) {
            return Err(format!(
                "可执行文件覆盖路径不是可执行的普通文件：{}",
                path.display()
            ));
        }
        return Ok((path, ProgramSource::SettingsOverride));
    }
    find_executable(candidates)
        .map(|path| (path, ProgramSource::PathSearch))
        .ok_or_else(|| format!("未在 PATH 中找到 {}", candidates.join(" 或 ")))
}

pub(super) async fn command_output(program: &Path, args: &[&str]) -> Option<std::process::Output> {
    tokio::time::timeout(
        std::time::Duration::from_secs(CAPABILITY_PROBE_TIMEOUT_SECS),
        tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()
}

async fn run_process_probe_with_timeout(
    spec: ProcessSpec,
    current_dir: &Path,
    timeout_secs: u64,
) -> Result<std::process::Output, EngineFailureKind> {
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(current_dir)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(if spec.stdin_payload.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
    for (key, value) in &spec.environment {
        command.env(key, value);
    }
    for key in &spec.environment_remove {
        command.env_remove(key);
    }
    let mut child = command
        .spawn()
        .map_err(|_| EngineFailureKind::ProcessCrash)?;
    if let Some(payload) = spec.stdin_payload {
        let mut stdin = child.stdin.take().ok_or(EngineFailureKind::ProcessCrash)?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|_| EngineFailureKind::ProcessCrash)?;
        stdin
            .shutdown()
            .await
            .map_err(|_| EngineFailureKind::ProcessCrash)?;
    }
    tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
        .await
        .map_err(|_| EngineFailureKind::Timeout)?
        .map_err(|_| EngineFailureKind::ProcessCrash)
}

pub(super) async fn run_minimal_process_probe(
    spec: ProcessSpec,
    current_dir: &Path,
) -> Result<(), EngineFailureKind> {
    let output_protocol = spec.output_protocol;
    let output =
        run_process_probe_with_timeout(spec, current_dir, ONLINE_AUTH_PROBE_TIMEOUT_SECS).await?;
    if !output.status.success() {
        return Err(super::classify_process_failure(
            output.status.code(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    if minimal_probe_response_is_valid(output_protocol, &output.stdout) {
        Ok(())
    } else {
        Err(EngineFailureKind::ProtocolError)
    }
}

fn json_event_text(value: &serde_json::Value) -> Option<&str> {
    let event_type = value.get("type").and_then(serde_json::Value::as_str);
    if event_type == Some("thought") {
        return None;
    }
    if event_type == Some("text") {
        return value.get("data").and_then(serde_json::Value::as_str);
    }
    for field in ["content", "text", "message", "result", "response", "data"] {
        if let Some(text) = value.get(field).and_then(serde_json::Value::as_str) {
            return Some(text);
        }
    }
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
}

fn minimal_probe_response_is_valid(
    protocol: super::contract::OutputProtocol,
    stdout: &[u8],
) -> bool {
    match protocol {
        super::contract::OutputProtocol::RawText => {
            std::str::from_utf8(stdout).is_ok_and(|text| text.trim() == "OK")
        }
        super::contract::OutputProtocol::JsonLines => {
            let Ok(stdout) = std::str::from_utf8(stdout) else {
                return false;
            };
            let mut response = String::new();
            let mut saw_event = false;
            for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                    return false;
                };
                saw_event = true;
                if value.get("type").and_then(serde_json::Value::as_str) == Some("error") {
                    return false;
                }
                if let Some(text) = json_event_text(&value) {
                    response.push_str(text);
                }
            }
            saw_event && response.trim() == "OK"
        }
    }
}

fn executable_fingerprint(path: &Path) -> String {
    let Ok(metadata) = std::fs::metadata(path) else {
        return "metadata-unavailable".to_string();
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}:{modified}", metadata.len())
}

fn auth_probe_protocol(provider: &ExecutionProvider) -> &'static str {
    match provider {
        ExecutionProvider::ClaudeCode | ExecutionProvider::Codex => "raw-text",
        ExecutionProvider::KimiCli | ExecutionProvider::GrokBuild => "json-lines",
    }
}

fn auth_cache_key(provider: &ExecutionProvider, path: &Path, capabilities: &[String]) -> String {
    format!(
        "{:?}:{}:{}:{}:{}:{}",
        provider,
        path.to_string_lossy(),
        executable_fingerprint(path),
        AUTH_PROBE_CONTRACT_VERSION,
        auth_probe_protocol(provider),
        capabilities.join(",")
    )
}

fn cached_authentication(
    provider: &ExecutionProvider,
    path: &Path,
    capabilities: &[String],
) -> Option<EngineAuthenticationResult> {
    let key = auth_cache_key(provider, path, capabilities);
    let mut cache = AUTH_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    let cached = cache.get(&key)?.clone();
    if cached.expires_at <= Instant::now()
        || cached.result.online_state != EngineOnlineAuthState::Verified
    {
        cache.remove(&key);
        return None;
    }
    Some(cached.result)
}

fn cache_authentication(
    provider: &ExecutionProvider,
    path: &Path,
    capabilities: &[String],
    result: EngineAuthenticationResult,
) {
    if result.online_state != EngineOnlineAuthState::Verified {
        return;
    }
    let key = auth_cache_key(provider, path, capabilities);
    if let Ok(mut cache) = AUTH_CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        cache.insert(
            key,
            CachedAuthentication {
                result,
                expires_at: Instant::now() + Duration::from_secs(AUTH_RESULT_TTL_SECS),
            },
        );
    }
}

async fn verified_capabilities(
    provider: &ExecutionProvider,
    path: &Path,
) -> Result<Vec<String>, String> {
    let mut capabilities = match provider {
        ExecutionProvider::ClaudeCode => super::claude_code::capability_probe(path).await?,
        ExecutionProvider::Codex => super::codex::capability_probe(path).await?,
        ExecutionProvider::KimiCli => super::kimi_cli::capability_probe(path).await?,
        ExecutionProvider::GrokBuild => super::grok_cli::capability_probe(path).await?,
    };
    capabilities.push("uses-user-default-model".to_string());
    Ok(capabilities)
}

fn auth_state(authentication: &EngineAuthenticationResult) -> EngineAuthState {
    match authentication.online_state {
        EngineOnlineAuthState::Verified => EngineAuthState::Authenticated,
        EngineOnlineAuthState::Failed
            if authentication.failure_kind == Some(EngineFailureKind::AuthenticationError) =>
        {
            EngineAuthState::Unauthenticated
        }
        _ if authentication.local_state == EngineLocalAuthState::Missing => {
            EngineAuthState::Unauthenticated
        }
        _ => EngineAuthState::Unknown,
    }
}

fn health_status(authentication: &EngineAuthenticationResult) -> EngineHealthStatus {
    match authentication.online_state {
        EngineOnlineAuthState::Verified => EngineHealthStatus::Available,
        EngineOnlineAuthState::Failed => EngineHealthStatus::VerificationFailed,
        EngineOnlineAuthState::NotVerified => match authentication.local_state {
            EngineLocalAuthState::ConfiguredEvidence | EngineLocalAuthState::Unknown => {
                EngineHealthStatus::VerificationRequired
            }
            EngineLocalAuthState::Missing => EngineHealthStatus::Unauthenticated,
        },
    }
}

fn verification_failure_message(kind: &EngineFailureKind) -> &'static str {
    match kind {
        EngineFailureKind::AuthenticationError => "认证失败",
        EngineFailureKind::QuotaExceeded => "额度不足",
        EngineFailureKind::RateLimited => "请求被限流",
        EngineFailureKind::ProviderUnavailable => "服务暂不可用",
        EngineFailureKind::NetworkError => "网络错误",
        EngineFailureKind::Timeout => "验证超时",
        EngineFailureKind::ProcessCrash => "CLI 进程异常",
        EngineFailureKind::ToolRejected => "工具权限被拒绝",
        EngineFailureKind::ProtocolError => "执行协议错误",
        EngineFailureKind::OutputTruncated => "模型输出截断",
        EngineFailureKind::MaxTurnsExceeded => "执行轮数已耗尽",
        EngineFailureKind::RuntimeError => "执行运行时错误",
        EngineFailureKind::TaskExecutionError => "验证请求失败",
    }
}

fn unavailable_health(
    profile: &ExecutionProfile,
    status: EngineHealthStatus,
    configuration_valid: bool,
    message: String,
) -> HealthCheckResult {
    HealthCheckResult {
        health: EngineHealth {
            runtime: profile.runtime.clone(),
            provider: profile.provider.clone(),
            status,
            executable_path: None,
            version: None,
            auth_state: EngineAuthState::Unknown,
            authentication: EngineAuthenticationResult::unknown("尚未获得执行引擎认证信息"),
            supports_unattended: false,
            configuration_valid,
            capabilities: vec![],
            source_revision: None,
            runtime_self_test: Default::default(),
            message,
        },
        program: None,
        program_source: None,
    }
}

pub(super) fn settings_failure(profile: &ExecutionProfile, message: String) -> EngineHealth {
    unavailable_health(profile, EngineHealthStatus::Unknown, false, message).health
}

pub(super) async fn check_engine_health_with_settings(
    profile: &ExecutionProfile,
    settings: &AppSettings,
    built_in_api_key: Option<&str>,
) -> HealthCheckResult {
    if profile.runtime == ExecutionRuntime::BuiltIn {
        return HealthCheckResult {
            health: super::builtin::health(settings, built_in_api_key),
            program: None,
            program_source: None,
        };
    }

    let (configured, candidates) = executable_configuration(&profile.provider, settings);
    let (path, program_source) = match resolve_executable(configured, candidates) {
        Ok(resolved) => resolved,
        Err(message) => {
            return unavailable_health(
                profile,
                EngineHealthStatus::NotInstalled,
                configured.is_none(),
                message,
            );
        }
    };
    let version = command_output(&path, &["--version"])
        .await
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|version| !version.is_empty());

    let capabilities = match verified_capabilities(&profile.provider, &path).await {
        Ok(capabilities) => capabilities,
        Err(message) => {
            let mut result = unavailable_health(
                profile,
                EngineHealthStatus::UnsupportedVersion,
                true,
                message,
            );
            result.health.executable_path = Some(path.to_string_lossy().to_string());
            result.health.version = version;
            result.program = Some(path.into_os_string());
            result.program_source = Some(program_source);
            return result;
        }
    };

    let authentication = if let Some(cached) =
        cached_authentication(&profile.provider, &path, &capabilities)
    {
        cached
    } else {
        match profile.provider {
            ExecutionProvider::KimiCli => {
                let (local_state, message) = super::kimi_cli::passive_auth_probe(&path).await;
                EngineAuthenticationResult {
                    local_state,
                    online_state: EngineOnlineAuthState::NotVerified,
                    method: EngineAuthVerificationMethod::PassiveConfiguration,
                    verified_at: None,
                    expires_at: None,
                    failure_kind: None,
                    message,
                }
            }
            ExecutionProvider::GrokBuild => {
                let (local_state, message) = super::grok_cli::passive_auth_probe(&path).await;
                EngineAuthenticationResult {
                    local_state,
                    online_state: EngineOnlineAuthState::NotVerified,
                    method: EngineAuthVerificationMethod::PassiveConfiguration,
                    verified_at: None,
                    expires_at: None,
                    failure_kind: None,
                    message,
                }
            }
            ExecutionProvider::ClaudeCode | ExecutionProvider::Codex => {
                let auth_output = if profile.provider == ExecutionProvider::ClaudeCode {
                    command_output(&path, &["auth", "status"]).await
                } else {
                    command_output(&path, &["login", "status"]).await
                };
                let authenticated = match (&profile.provider, auth_output) {
                    (ExecutionProvider::ClaudeCode, Some(output)) if output.status.success() => {
                        serde_json::from_slice::<serde_json::Value>(&output.stdout)
                            .ok()
                            .and_then(|item| item.get("loggedIn")?.as_bool())
                    }
                    (ExecutionProvider::Codex, Some(output)) if output.status.success() => {
                        Some(true)
                    }
                    (_, Some(output)) if !output.status.success() => Some(false),
                    _ => None,
                };
                EngineAuthenticationResult {
                    local_state: match authenticated {
                        Some(true) => EngineLocalAuthState::ConfiguredEvidence,
                        Some(false) => EngineLocalAuthState::Missing,
                        None => EngineLocalAuthState::Unknown,
                    },
                    online_state: EngineOnlineAuthState::NotVerified,
                    method: EngineAuthVerificationMethod::PassiveConfiguration,
                    verified_at: None,
                    expires_at: None,
                    failure_kind: None,
                    message: match authenticated {
                        Some(true) => format!("{} 已认证", profile.provider.display_name()),
                        Some(false) => format!("{} 尚未认证", profile.provider.display_name()),
                        None => format!("{} 认证状态未知", profile.provider.display_name()),
                    },
                }
            }
        }
    };
    let auth_state = auth_state(&authentication);
    let status = health_status(&authentication);
    let message = authentication.message.clone();
    HealthCheckResult {
        health: EngineHealth {
            runtime: profile.runtime.clone(),
            provider: profile.provider.clone(),
            status,
            executable_path: Some(path.to_string_lossy().to_string()),
            version,
            auth_state,
            authentication,
            supports_unattended: true,
            configuration_valid: true,
            capabilities,
            source_revision: None,
            runtime_self_test: Default::default(),
            message,
        },
        program: Some(path.into_os_string()),
        program_source: Some(program_source),
    }
}

pub(super) async fn verify_engine_authentication_with_settings(
    profile: &ExecutionProfile,
    settings: &AppSettings,
) -> Result<EngineAuthenticationResult, String> {
    if profile.runtime != ExecutionRuntime::Plugin {
        return Err("只有外部 CLI 插件支持主动认证验证".to_string());
    }
    let (configured, candidates) = executable_configuration(&profile.provider, settings);
    let (path, _) = resolve_executable(configured, candidates)?;
    let capabilities = verified_capabilities(&profile.provider, &path).await?;
    let (local_state, local_message) = match profile.provider {
        ExecutionProvider::KimiCli => super::kimi_cli::passive_auth_probe(&path).await,
        ExecutionProvider::GrokBuild => super::grok_cli::passive_auth_probe(&path).await,
        ExecutionProvider::ClaudeCode => {
            let output = command_output(&path, &["auth", "status"]).await;
            match output {
                Some(output) if output.status.success() => (
                    EngineLocalAuthState::ConfiguredEvidence,
                    "Claude Code 已发现本地认证配置".to_string(),
                ),
                Some(_) => (
                    EngineLocalAuthState::Missing,
                    "Claude Code 尚未认证".to_string(),
                ),
                None => (
                    EngineLocalAuthState::Unknown,
                    "Claude Code 认证状态未知".to_string(),
                ),
            }
        }
        ExecutionProvider::Codex => {
            let output = command_output(&path, &["login", "status"]).await;
            match output {
                Some(output) if output.status.success() => (
                    EngineLocalAuthState::ConfiguredEvidence,
                    "Codex 已发现本地认证配置".to_string(),
                ),
                Some(_) => (EngineLocalAuthState::Missing, "Codex 尚未认证".to_string()),
                None => (
                    EngineLocalAuthState::Unknown,
                    "Codex 认证状态未知".to_string(),
                ),
            }
        }
    };
    let directory = AuthProbeDirectory::new()?;
    let online = match profile.provider {
        ExecutionProvider::ClaudeCode => {
            super::claude_code::online_auth_probe(&path, &directory.0).await
        }
        ExecutionProvider::Codex => super::codex::online_auth_probe(&path, &directory.0).await,
        ExecutionProvider::KimiCli => super::kimi_cli::online_auth_probe(&path, &directory.0).await,
        ExecutionProvider::GrokBuild => {
            super::grok_cli::online_auth_probe(&path, &directory.0).await
        }
    };
    let verified_at = chrono::Utc::now();
    let expires_at = verified_at + chrono::Duration::seconds(AUTH_RESULT_TTL_SECS as i64);
    let result = match online {
        Ok(method) => EngineAuthenticationResult {
            local_state,
            online_state: EngineOnlineAuthState::Verified,
            method,
            verified_at: Some(verified_at.to_rfc3339()),
            expires_at: Some(expires_at.to_rfc3339()),
            failure_kind: None,
            message: format!("{} 在线认证验证成功", profile.provider.display_name()),
        },
        Err(kind) => EngineAuthenticationResult {
            local_state,
            online_state: EngineOnlineAuthState::Failed,
            method: EngineAuthVerificationMethod::OnlineMinimalRequest,
            verified_at: Some(verified_at.to_rfc3339()),
            expires_at: Some(expires_at.to_rfc3339()),
            message: format!(
                "{} 在线认证验证失败：{}；本地状态：{}",
                profile.provider.display_name(),
                verification_failure_message(&kind),
                local_message
            ),
            failure_kind: Some(kind),
        },
    };
    cache_authentication(&profile.provider, &path, &capabilities, result.clone());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PermissionProfile;
    use std::ffi::OsString;

    #[cfg(unix)]
    fn write_fake_cli(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("应能写入假 CLI");
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    fn fake_probe_spec(program: &Path, args: &[&str], stdin: Option<&str>) -> ProcessSpec {
        fake_probe_spec_with_protocol(
            program,
            args,
            stdin,
            super::super::contract::OutputProtocol::RawText,
        )
    }

    #[cfg(unix)]
    fn fake_probe_spec_with_protocol(
        program: &Path,
        args: &[&str],
        stdin: Option<&str>,
        output_protocol: super::super::contract::OutputProtocol,
    ) -> ProcessSpec {
        ProcessSpec {
            display_name: "Fake CLI",
            program: program.as_os_str().to_owned(),
            args: args.iter().map(OsString::from).collect(),
            stdin_payload: stdin.map(str::to_string),
            environment: vec![],
            environment_remove: vec![],
            output_protocol,
            program_source: ProgramSource::SettingsOverride,
            timeout_secs: 1,
        }
    }

    #[tokio::test]
    async fn builtin_engine_requires_a_metheus_managed_secret() {
        let result = check_engine_health_with_settings(
            &ExecutionProfile {
                runtime: ExecutionRuntime::BuiltIn,
                provider: ExecutionProvider::GrokBuild,
                permission_profile: PermissionProfile::Unattended,
                profile_revision: 1,
            },
            &AppSettings::default(),
            None,
        )
        .await;
        let built_in_compiled = super::super::builtin::is_compiled();
        let expected_status = if built_in_compiled {
            EngineHealthStatus::Unauthenticated
        } else {
            EngineHealthStatus::Disabled
        };
        assert_eq!(result.health.status, expected_status);
        assert!(result.health.status.blocks_execution());
        assert_eq!(result.health.supports_unattended, built_in_compiled);
        assert!(result.health.executable_path.is_none());
    }

    #[test]
    fn only_known_unusable_health_states_block_execution() {
        assert!(EngineHealthStatus::NotInstalled.blocks_execution());
        assert!(EngineHealthStatus::Unauthenticated.blocks_execution());
        assert!(EngineHealthStatus::UnsupportedVersion.blocks_execution());
        assert!(EngineHealthStatus::Disabled.blocks_execution());
        assert!(EngineHealthStatus::VerificationRequired.blocks_execution());
        assert!(EngineHealthStatus::VerificationFailed.blocks_execution());
        assert!(!EngineHealthStatus::Available.blocks_execution());
        assert!(!EngineHealthStatus::Unknown.blocks_execution());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn minimal_probe_executes_exact_arguments_and_stdin_in_empty_directory() {
        let directory = AuthProbeDirectory::new().unwrap();
        let cli = directory.0.join("fake-success");
        write_fake_cli(
            &cli,
            "test \"$1\" = '--approved'\ntest \"$(cat)\" = 'probe prompt'\nprintf 'OK\\n'",
        );
        let result = run_process_probe_with_timeout(
            fake_probe_spec(&cli, &["--approved"], Some("probe prompt")),
            &directory.0,
            2,
        )
        .await
        .unwrap();
        assert!(result.status.success());
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "OK");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn minimal_probe_rejects_non_protocol_output_and_accepts_exact_semantics() {
        let directory = AuthProbeDirectory::new().unwrap();

        let raw_ok = directory.0.join("raw-ok");
        write_fake_cli(&raw_ok, "printf 'OK\\n'");
        assert_eq!(
            run_minimal_process_probe(fake_probe_spec(&raw_ok, &[], None), &directory.0).await,
            Ok(())
        );

        for (name, body) in [
            ("help", "printf 'Usage: fake-cli [OPTIONS]\\n'"),
            ("empty", ":"),
            ("arbitrary", "printf 'authenticated maybe\\n'"),
        ] {
            let cli = directory.0.join(name);
            write_fake_cli(&cli, body);
            assert_eq!(
                run_minimal_process_probe(fake_probe_spec(&cli, &[], None), &directory.0).await,
                Err(EngineFailureKind::ProtocolError),
                "{name} 输出必须 fail closed"
            );
        }

        let malformed_json = directory.0.join("malformed-json");
        write_fake_cli(&malformed_json, "printf '{broken\\n'");
        assert_eq!(
            run_minimal_process_probe(
                fake_probe_spec_with_protocol(
                    &malformed_json,
                    &[],
                    None,
                    super::super::contract::OutputProtocol::JsonLines,
                ),
                &directory.0,
            )
            .await,
            Err(EngineFailureKind::ProtocolError)
        );

        let json_help = directory.0.join("json-help");
        write_fake_cli(&json_help, "printf '%s\\n' '{\"help\":\"OK\"}'");
        assert_eq!(
            run_minimal_process_probe(
                fake_probe_spec_with_protocol(
                    &json_help,
                    &[],
                    None,
                    super::super::contract::OutputProtocol::JsonLines,
                ),
                &directory.0,
            )
            .await,
            Err(EngineFailureKind::ProtocolError)
        );

        let json_ok = directory.0.join("json-ok");
        write_fake_cli(
            &json_ok,
            "printf '%s\\n' '{\"type\":\"text\",\"data\":\"OK\"}' '{\"type\":\"end\"}'",
        );
        assert_eq!(
            run_minimal_process_probe(
                fake_probe_spec_with_protocol(
                    &json_ok,
                    &[],
                    None,
                    super::super::contract::OutputProtocol::JsonLines,
                ),
                &directory.0,
            )
            .await,
            Ok(())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn minimal_probe_classifies_authentication_failure_and_timeout() {
        let directory = AuthProbeDirectory::new().unwrap();
        let auth_cli = directory.0.join("fake-auth-failure");
        write_fake_cli(&auth_cli, "printf 'authentication failed\\n' >&2\nexit 1");
        let failure =
            run_minimal_process_probe(fake_probe_spec(&auth_cli, &[], None), &directory.0).await;
        assert_eq!(failure, Err(EngineFailureKind::AuthenticationError));

        let slow_cli = directory.0.join("fake-timeout");
        write_fake_cli(&slow_cli, "exec sleep 5");
        let timeout =
            run_process_probe_with_timeout(fake_probe_spec(&slow_cli, &[], None), &directory.0, 1)
                .await;
        assert!(matches!(timeout, Err(EngineFailureKind::Timeout)));
    }

    #[cfg(unix)]
    #[test]
    fn authentication_cache_is_bound_to_verified_contract_and_capabilities() {
        let directory = AuthProbeDirectory::new().unwrap();
        let cli = directory.0.join("cache-cli");
        write_fake_cli(&cli, "printf 'OK\\n'");
        let capabilities = vec!["unattended".to_string(), "non-interactive".to_string()];
        let key = auth_cache_key(&ExecutionProvider::ClaudeCode, &cli, &capabilities);
        assert!(key.contains(AUTH_PROBE_CONTRACT_VERSION));
        assert_ne!(
            key,
            auth_cache_key(&ExecutionProvider::Codex, &cli, &capabilities)
        );
        assert_ne!(
            key,
            auth_cache_key(
                &ExecutionProvider::ClaudeCode,
                &cli,
                &["different-capability".to_string()],
            )
        );

        let failed = EngineAuthenticationResult {
            local_state: EngineLocalAuthState::ConfiguredEvidence,
            online_state: EngineOnlineAuthState::Failed,
            method: EngineAuthVerificationMethod::OnlineMinimalRequest,
            verified_at: None,
            expires_at: None,
            failure_kind: Some(EngineFailureKind::ProtocolError),
            message: "脱敏失败".to_string(),
        };
        cache_authentication(&ExecutionProvider::ClaudeCode, &cli, &capabilities, failed);
        assert!(
            cached_authentication(&ExecutionProvider::ClaudeCode, &cli, &capabilities).is_none()
        );

        let verified = EngineAuthenticationResult {
            local_state: EngineLocalAuthState::ConfiguredEvidence,
            online_state: EngineOnlineAuthState::Verified,
            method: EngineAuthVerificationMethod::OnlineMinimalRequest,
            verified_at: None,
            expires_at: None,
            failure_kind: None,
            message: "验证成功".to_string(),
        };
        cache_authentication(
            &ExecutionProvider::ClaudeCode,
            &cli,
            &capabilities,
            verified,
        );
        assert_eq!(
            cached_authentication(&ExecutionProvider::ClaudeCode, &cli, &capabilities)
                .unwrap()
                .online_state,
            EngineOnlineAuthState::Verified
        );

        let changed_cli = directory.0.join("changed-cache-cli");
        write_fake_cli(&changed_cli, "printf 'OK\\n'");
        assert_ne!(
            key,
            auth_cache_key(&ExecutionProvider::ClaudeCode, &changed_cli, &capabilities,)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verified_cache_cannot_bypass_a_changed_capability_probe() {
        let directory = AuthProbeDirectory::new().unwrap();
        let cli = directory.0.join("fake-claude");
        write_fake_cli(
            &cli,
            "case \"${1:-}\" in\n  --version) printf '1.0\\n' ;;\n  --help) printf '%s\\n' '--dangerously-skip-permissions -p' ;;\n  auth) printf '%s\\n' '{\"loggedIn\":true}' ;;\n  --dangerously-skip-permissions) printf 'OK\\n' ;;\n  *) exit 2 ;;\nesac",
        );
        let mut settings = AppSettings::default();
        settings.plugin_cli.claude_code_path = Some(cli.to_string_lossy().to_string());
        let profile = ExecutionProfile {
            runtime: ExecutionRuntime::Plugin,
            provider: ExecutionProvider::ClaudeCode,
            permission_profile: PermissionProfile::Unattended,
            profile_revision: 1,
        };

        let verified = verify_engine_authentication_with_settings(&profile, &settings)
            .await
            .unwrap();
        assert_eq!(verified.online_state, EngineOnlineAuthState::Verified);
        let available = check_engine_health_with_settings(&profile, &settings, None).await;
        assert_eq!(available.health.status, EngineHealthStatus::Available);

        write_fake_cli(
            &cli,
            "case \"${1:-}\" in\n  --version) printf '1.1\\n' ;;\n  --help) printf 'usage only\\n' ;;\n  auth) printf '%s\\n' '{\"loggedIn\":true}' ;;\n  --dangerously-skip-permissions) printf 'OK\\n' ;;\n  *) exit 2 ;;\nesac",
        );
        let changed = check_engine_health_with_settings(&profile, &settings, None).await;
        assert_eq!(
            changed.health.status,
            EngineHealthStatus::UnsupportedVersion
        );
        assert_ne!(changed.health.status, EngineHealthStatus::Available);
    }
}
