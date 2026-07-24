use crate::project::{EngineFailureKind, ExecutionProvider, ExecutionRuntime};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;

#[derive(Debug, Clone)]
pub(crate) struct ExecutionRequest {
    pub project_path: String,
    pub prompt: String,
    pub authorized_paths: Vec<String>,
    pub subtask_id: String,
    pub execution_id: String,
}

#[derive(Debug)]
pub(crate) enum EngineError {
    NotInstalled(String),
    Unavailable(String),
    InvalidConfiguration(String),
    StartFailed(String),
    Timeout,
    ProcessFailed(String),
    Cancelled,
    ProtocolError(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled(message)
            | Self::Unavailable(message)
            | Self::InvalidConfiguration(message)
            | Self::StartFailed(message)
            | Self::ProcessFailed(message)
            | Self::ProtocolError(message) => formatter.write_str(message),
            Self::Timeout => formatter.write_str("执行超时"),
            Self::Cancelled => formatter.write_str("执行已暂停"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputProtocol {
    RawText,
    JsonLines,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgramSource {
    PathSearch,
    SettingsOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineHealthStatus {
    Available,
    NotInstalled,
    Unauthenticated,
    UnsupportedVersion,
    Disabled,
    VerificationRequired,
    VerificationFailed,
    Unknown,
}

impl EngineHealthStatus {
    pub(crate) fn blocks_execution(&self) -> bool {
        matches!(
            self,
            Self::NotInstalled
                | Self::Unauthenticated
                | Self::UnsupportedVersion
                | Self::Disabled
                | Self::VerificationRequired
                | Self::VerificationFailed
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum EngineRuntimeSelfTestState {
    #[default]
    NotRun,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EngineRuntimeSelfTestResult {
    pub success: bool,
    pub state: EngineRuntimeSelfTestState,
    pub source_revision: String,
    pub verified_at: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineAuthState {
    Authenticated,
    Unauthenticated,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineLocalAuthState {
    ConfiguredEvidence,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineOnlineAuthState {
    NotVerified,
    Verified,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineAuthVerificationMethod {
    None,
    PassiveConfiguration,
    OnlineMinimalRequest,
    OnlineModelList,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EngineAuthenticationResult {
    pub local_state: EngineLocalAuthState,
    pub online_state: EngineOnlineAuthState,
    pub method: EngineAuthVerificationMethod,
    pub verified_at: Option<String>,
    pub expires_at: Option<String>,
    pub failure_kind: Option<EngineFailureKind>,
    pub message: String,
}

impl EngineAuthenticationResult {
    pub(crate) fn unknown(message: impl Into<String>) -> Self {
        Self {
            local_state: EngineLocalAuthState::Unknown,
            online_state: EngineOnlineAuthState::NotVerified,
            method: EngineAuthVerificationMethod::None,
            verified_at: None,
            expires_at: None,
            failure_kind: None,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EngineHealth {
    pub runtime: ExecutionRuntime,
    pub provider: ExecutionProvider,
    pub status: EngineHealthStatus,
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub auth_state: EngineAuthState,
    pub authentication: EngineAuthenticationResult,
    pub supports_unattended: bool,
    pub configuration_valid: bool,
    pub capabilities: Vec<String>,
    pub source_revision: Option<String>,
    pub runtime_self_test: EngineRuntimeSelfTestState,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(super) struct ProcessSpec {
    pub display_name: &'static str,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub stdin_payload: Option<String>,
    pub environment: Vec<(OsString, OsString)>,
    pub environment_remove: Vec<OsString>,
    pub output_protocol: OutputProtocol,
    pub program_source: ProgramSource,
    pub timeout_secs: u64,
}

#[derive(Debug)]
pub(super) struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}
