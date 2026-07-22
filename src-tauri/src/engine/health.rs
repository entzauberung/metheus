use super::contract::{EngineAuthState, EngineHealth, EngineHealthStatus};
use crate::project::{ExecutionProfile, ExecutionProvider, ExecutionRuntime};
use std::path::{Path, PathBuf};

fn executable_name(provider: &ExecutionProvider) -> Option<&'static str> {
    match provider {
        ExecutionProvider::ClaudeCode => Some("claude"),
        ExecutionProvider::Codex => Some("codex"),
        ExecutionProvider::GrokBuild => None,
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat"] {
            let candidate = directory.join(format!("{name}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

async fn command_output(program: &Path, args: &[&str]) -> Option<std::process::Output> {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()
}

pub(crate) async fn check_engine_health(profile: &ExecutionProfile) -> EngineHealth {
    if profile.runtime == ExecutionRuntime::BuiltIn {
        return super::builtin::health();
    }

    let Some(name) = executable_name(&profile.provider) else {
        return EngineHealth {
            provider: profile.provider.clone(),
            status: EngineHealthStatus::Disabled,
            executable_path: None,
            version: None,
            auth_state: EngineAuthState::Unknown,
            supports_unattended: false,
            message: "该执行引擎尚未启用".to_string(),
        };
    };
    let Some(path) = find_executable(name) else {
        return EngineHealth {
            provider: profile.provider.clone(),
            status: EngineHealthStatus::NotInstalled,
            executable_path: None,
            version: None,
            auth_state: EngineAuthState::Unknown,
            supports_unattended: true,
            message: format!("未在 PATH 中找到 {}", profile.provider.display_name()),
        };
    };

    let version = command_output(&path, &["--version"])
        .await
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|version| !version.is_empty());
    let auth_output = match profile.provider {
        ExecutionProvider::ClaudeCode => command_output(&path, &["auth", "status"]).await,
        ExecutionProvider::Codex => command_output(&path, &["login", "status"]).await,
        ExecutionProvider::GrokBuild => None,
    };
    let auth_state = match (&profile.provider, auth_output) {
        (ExecutionProvider::ClaudeCode, Some(output)) if output.status.success() => {
            let value: Option<serde_json::Value> = serde_json::from_slice(&output.stdout).ok();
            match value.and_then(|item| item.get("loggedIn").and_then(|value| value.as_bool())) {
                Some(true) => EngineAuthState::Authenticated,
                Some(false) => EngineAuthState::Unauthenticated,
                None => EngineAuthState::Unknown,
            }
        }
        (ExecutionProvider::Codex, Some(output)) if output.status.success() => {
            EngineAuthState::Authenticated
        }
        (_, Some(output)) if !output.status.success() => EngineAuthState::Unauthenticated,
        _ => EngineAuthState::Unknown,
    };
    let status = match auth_state {
        EngineAuthState::Authenticated => EngineHealthStatus::Available,
        EngineAuthState::Unauthenticated => EngineHealthStatus::Unauthenticated,
        EngineAuthState::Unknown => EngineHealthStatus::Unknown,
    };
    let message = match status {
        EngineHealthStatus::Available => format!("{} 已就绪", profile.provider.display_name()),
        EngineHealthStatus::Unauthenticated => {
            format!("{} 尚未认证", profile.provider.display_name())
        }
        _ => format!("无法确认 {} 的认证状态", profile.provider.display_name()),
    };
    EngineHealth {
        provider: profile.provider.clone(),
        status,
        executable_path: Some(path.to_string_lossy().to_string()),
        version,
        auth_state,
        supports_unattended: true,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ExecutionRuntime, PermissionProfile};

    #[tokio::test]
    async fn builtin_engine_is_explicitly_disabled() {
        let health = check_engine_health(&ExecutionProfile {
            runtime: ExecutionRuntime::BuiltIn,
            provider: ExecutionProvider::GrokBuild,
            permission_profile: PermissionProfile::Unattended,
            profile_revision: 1,
        })
        .await;
        assert_eq!(health.status, EngineHealthStatus::Disabled);
        assert!(!health.supports_unattended);
    }

    #[test]
    fn only_known_unusable_health_states_block_execution() {
        assert!(EngineHealthStatus::NotInstalled.blocks_execution());
        assert!(EngineHealthStatus::Unauthenticated.blocks_execution());
        assert!(EngineHealthStatus::UnsupportedVersion.blocks_execution());
        assert!(EngineHealthStatus::Disabled.blocks_execution());
        assert!(!EngineHealthStatus::Available.blocks_execution());
        assert!(!EngineHealthStatus::Unknown.blocks_execution());
    }
}
