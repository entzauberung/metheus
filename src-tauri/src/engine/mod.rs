#[cfg(feature = "builtin-grok")]
mod builtin;
#[cfg(not(feature = "builtin-grok"))]
#[path = "builtin_disabled.rs"]
mod builtin;
mod claude_code;
mod codex;
mod contract;
mod failure_classifier;
mod grok_cli;
mod health;
mod kimi_cli;
mod process_runner;
mod service;

#[cfg(test)]
pub(crate) use contract::{EngineAuthState, EngineHealthStatus};
pub(crate) use contract::{
    EngineAuthenticationResult, EngineError, EngineHealth, EngineRuntimeSelfTestResult,
    ExecutionRequest,
};
pub(crate) use failure_classifier::{
    blocks_code_recovery, classify_process_failure, requires_human_recovery,
};
pub(crate) use service::{
    check_engine_health, execute, prepare_engine, validate_profile, verify_engine_authentication,
    PreparedEngine,
};

pub(crate) async fn test_grok_build_runtime() -> EngineRuntimeSelfTestResult {
    debug_assert_eq!(builtin_grok_compiled(), cfg!(feature = "builtin-grok"));
    builtin::test_runtime().await
}

pub(crate) async fn test_builtin_grok_model_connection() -> crate::settings::ConnectionTestResult {
    builtin::test_model_connection().await
}

pub(crate) fn builtin_grok_source_revision() -> Option<String> {
    builtin::source_revision()
}

pub(crate) const fn builtin_grok_compiled() -> bool {
    builtin::is_compiled()
}
