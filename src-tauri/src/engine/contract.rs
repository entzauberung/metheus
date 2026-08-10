use crate::project::{EngineFailureKind, ExecutionProvider, ExecutionResult, ExecutionRuntime};
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
    pub task_budget: crate::task_contract::TaskBudgetSummary,
}

#[derive(Debug)]
pub(crate) enum EngineError {
    NotInstalled(String),
    Unavailable(String),
    InvalidConfiguration(String),
    StartFailed(String),
    Timeout {
        execution_result: Option<Box<ExecutionResult>>,
    },
    ProcessFailed(String),
    Cancelled {
        execution_result: Option<Box<ExecutionResult>>,
    },
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
            Self::Timeout { .. } => formatter.write_str("执行超时"),
            Self::Cancelled { .. } => formatter.write_str("执行已暂停"),
        }
    }
}

impl EngineError {
    pub(crate) fn timeout() -> Self {
        Self::Timeout {
            execution_result: None,
        }
    }

    pub(crate) fn cancelled() -> Self {
        Self::Cancelled {
            execution_result: None,
        }
    }

    pub(crate) fn timeout_with_result(result: ExecutionResult) -> Self {
        Self::Timeout {
            execution_result: Some(Box::new(result)),
        }
    }

    pub(crate) fn cancelled_with_result(result: ExecutionResult) -> Self {
        Self::Cancelled {
            execution_result: Some(Box::new(result)),
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
        !matches!(self, Self::Available)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum EngineConfigurationEvidenceSource {
    Confirmed,
    ProviderDefault,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct EngineRuntimeConfigurationEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub model_source: EngineConfigurationEvidenceSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub reasoning_effort_source: EngineConfigurationEvidenceSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EngineAuthenticationResult {
    pub local_state: EngineLocalAuthState,
    pub online_state: EngineOnlineAuthState,
    pub method: EngineAuthVerificationMethod,
    pub verified_at: Option<String>,
    pub expires_at: Option<String>,
    pub failure_kind: Option<EngineFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_configuration: Option<EngineRuntimeConfigurationEvidence>,
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
            runtime_configuration: None,
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
