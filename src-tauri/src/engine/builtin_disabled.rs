use super::contract::{
    EngineAuthState, EngineAuthenticationResult, EngineError, EngineHealth, EngineHealthStatus,
    EngineRuntimeSelfTestResult, EngineRuntimeSelfTestState, ExecutionRequest,
};
use crate::pipeline::PipelineState;
use crate::project::{ExecutionProvider, ExecutionResult, ExecutionRuntime};
use crate::settings::{
    AppSettings, ConnectionTestResult, ModelConnectionErrorKind, ModelConnectionTarget,
};
use std::sync::Arc;
use tokio::sync::Mutex;

const BUILTIN_GROK_DISABLED_MESSAGE: &str = "当前为轻量开发构建，未包含预装 Grok Build";

pub(super) const fn is_compiled() -> bool {
    false
}

pub(super) fn source_revision() -> Option<String> {
    None
}

pub(super) fn health(_settings: &AppSettings, _api_key: Option<&str>) -> EngineHealth {
    EngineHealth {
        runtime: ExecutionRuntime::BuiltIn,
        provider: ExecutionProvider::GrokBuild,
        status: EngineHealthStatus::Disabled,
        executable_path: None,
        version: None,
        auth_state: EngineAuthState::Unknown,
        authentication: EngineAuthenticationResult::unknown(BUILTIN_GROK_DISABLED_MESSAGE),
        supports_unattended: false,
        configuration_valid: true,
        capabilities: vec![],
        source_revision: None,
        runtime_self_test: EngineRuntimeSelfTestState::NotRun,
        message: BUILTIN_GROK_DISABLED_MESSAGE.to_string(),
    }
}

pub(crate) async fn test_runtime() -> EngineRuntimeSelfTestResult {
    EngineRuntimeSelfTestResult {
        success: false,
        state: EngineRuntimeSelfTestState::Failed,
        source_revision: String::new(),
        verified_at: chrono::Utc::now().to_rfc3339(),
        message: BUILTIN_GROK_DISABLED_MESSAGE.to_string(),
    }
}

pub(super) async fn test_model_connection() -> ConnectionTestResult {
    ConnectionTestResult {
        success: false,
        target: ModelConnectionTarget::BuiltInGrokBuild,
        model: String::new(),
        latency_ms: 0,
        error_kind: Some(ModelConnectionErrorKind::InvalidConfiguration),
        message: BUILTIN_GROK_DISABLED_MESSAGE.to_string(),
    }
}

pub(super) async fn execute(
    _app_settings: &AppSettings,
    _api_key: &str,
    _request: ExecutionRequest,
    _state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<ExecutionResult, EngineError> {
    Err(EngineError::Unavailable(
        BUILTIN_GROK_DISABLED_MESSAGE.to_string(),
    ))
}
