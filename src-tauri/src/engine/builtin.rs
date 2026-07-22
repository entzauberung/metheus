use super::contract::{EngineAuthState, EngineError, EngineHealth, EngineHealthStatus};
use crate::project::ExecutionProvider;

pub(super) fn unavailable_error() -> EngineError {
    EngineError::Unavailable("Grok Build 预装引擎尚未启用，请选择插件模式".to_string())
}

pub(super) fn health() -> EngineHealth {
    EngineHealth {
        provider: ExecutionProvider::GrokBuild,
        status: EngineHealthStatus::Disabled,
        executable_path: None,
        version: None,
        auth_state: EngineAuthState::Unknown,
        supports_unattended: false,
        message: "Grok Build 预装引擎尚未启用".to_string(),
    }
}
