use super::contract::{
    EngineAuthState, EngineAuthVerificationMethod, EngineAuthenticationResult, EngineHealth,
    EngineHealthStatus, EngineLocalAuthState, EngineOnlineAuthState, ProgramSource,
};
use crate::project::{EngineFailureKind, ExecutionProfile, ExecutionProvider, ExecutionRuntime};
use crate::settings::AppSettings;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CAPABILITY_PROBE_TIMEOUT_SECS: u64 = 5;
const ONLINE_AUTH_PROBE_TIMEOUT_SECS: u64 = 30;
const AUTH_RESULT_TTL_SECS: u64 = 5 * 60;

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

pub(super) async fn online_command_output(
    program: &Path,
    args: &[&str],
    current_dir: &Path,
    environment_remove: &[&str],
) -> Result<std::process::Output, EngineFailureKind> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(current_dir)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    for key in environment_remove {
        command.env_remove(key);
    }
    tokio::time::timeout(
        Duration::from_secs(ONLINE_AUTH_PROBE_TIMEOUT_SECS),
        command.output(),
    )
    .await
    .map_err(|_| EngineFailureKind::Timeout)?
    .map_err(|_| EngineFailureKind::ProcessCrash)
}

fn auth_cache_key(provider: &ExecutionProvider, path: &Path) -> String {
    format!("{:?}:{}", provider, path.to_string_lossy())
}

fn cached_authentication(
    provider: &ExecutionProvider,
    path: &Path,
) -> Option<EngineAuthenticationResult> {
    let key = auth_cache_key(provider, path);
    let mut cache = AUTH_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;
    let cached = cache.get(&key)?.clone();
    if cached.expires_at <= Instant::now() {
        cache.remove(&key);
        return None;
    }
    Some(cached.result)
}

fn cache_authentication(
    provider: &ExecutionProvider,
    path: &Path,
    result: EngineAuthenticationResult,
) {
    let key = auth_cache_key(provider, path);
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
            EngineLocalAuthState::ConfiguredEvidence => EngineHealthStatus::Available,
            EngineLocalAuthState::Missing => EngineHealthStatus::Unauthenticated,
            EngineLocalAuthState::Unknown => EngineHealthStatus::Unknown,
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
            )
        }
    };
    let version = command_output(&path, &["--version"])
        .await
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|version| !version.is_empty());

    let capabilities = match profile.provider {
        ExecutionProvider::KimiCli => super::kimi_cli::capability_probe(&path).await,
        ExecutionProvider::GrokBuild => super::grok_cli::capability_probe(&path).await,
        ExecutionProvider::ClaudeCode | ExecutionProvider::Codex => Ok(vec![
            "unattended".to_string(),
            "non-interactive".to_string(),
        ]),
    };
    let capabilities = match capabilities {
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

    let authentication = match profile.provider {
        ExecutionProvider::KimiCli => {
            if let Some(cached) = cached_authentication(&profile.provider, &path) {
                cached
            } else {
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
        }
        ExecutionProvider::GrokBuild => {
            if let Some(cached) = cached_authentication(&profile.provider, &path) {
                cached
            } else {
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
                (ExecutionProvider::Codex, Some(output)) if output.status.success() => Some(true),
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
    if profile.runtime != ExecutionRuntime::Plugin
        || !matches!(
            profile.provider,
            ExecutionProvider::KimiCli | ExecutionProvider::GrokBuild
        )
    {
        return Err("仅 Kimi CLI 和 Grok Build CLI 支持主动认证验证".to_string());
    }
    let (configured, candidates) = executable_configuration(&profile.provider, settings);
    let (path, _) = resolve_executable(configured, candidates)?;
    let (local_state, local_message) = match profile.provider {
        ExecutionProvider::KimiCli => super::kimi_cli::passive_auth_probe(&path).await,
        ExecutionProvider::GrokBuild => super::grok_cli::passive_auth_probe(&path).await,
        _ => unreachable!(),
    };
    let directory = AuthProbeDirectory::new()?;
    let online = match profile.provider {
        ExecutionProvider::KimiCli => super::kimi_cli::online_auth_probe(&path, &directory.0).await,
        ExecutionProvider::GrokBuild => {
            super::grok_cli::online_auth_probe(&path, &directory.0).await
        }
        _ => unreachable!(),
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
            method: match profile.provider {
                ExecutionProvider::KimiCli => EngineAuthVerificationMethod::OnlineMinimalRequest,
                ExecutionProvider::GrokBuild => EngineAuthVerificationMethod::OnlineModelList,
                _ => EngineAuthVerificationMethod::None,
            },
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
    cache_authentication(&profile.provider, &path, result.clone());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::PermissionProfile;

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
        assert_eq!(result.health.status, EngineHealthStatus::Unauthenticated);
        assert!(result.health.supports_unattended);
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
}
