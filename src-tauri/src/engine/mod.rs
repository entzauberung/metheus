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
pub(crate) use failure_classifier::{blocks_code_recovery, classify_process_failure};
pub(crate) use service::{
    check_engine_health, execute, prepare_engine, validate_profile, verify_engine_authentication,
    PreparedEngine,
};

pub(crate) async fn test_grok_build_runtime() -> EngineRuntimeSelfTestResult {
    builtin::test_runtime().await
}
