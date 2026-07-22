use crate::project::ExecutionProvider;
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
    StartFailed(String),
    Timeout,
    ProcessFailed(String),
    Cancelled,
    ProtocolError(String),
    PermissionError(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled(message)
            | Self::Unavailable(message)
            | Self::StartFailed(message)
            | Self::ProcessFailed(message)
            | Self::ProtocolError(message)
            | Self::PermissionError(message) => formatter.write_str(message),
            Self::Timeout => formatter.write_str("执行超时"),
            Self::Cancelled => formatter.write_str("执行已暂停"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineHealthStatus {
    Available,
    NotInstalled,
    Unauthenticated,
    UnsupportedVersion,
    Disabled,
    Unknown,
}

impl EngineHealthStatus {
    pub(crate) fn blocks_execution(&self) -> bool {
        matches!(
            self,
            Self::NotInstalled | Self::Unauthenticated | Self::UnsupportedVersion | Self::Disabled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum EngineAuthState {
    Authenticated,
    Unauthenticated,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EngineHealth {
    pub provider: ExecutionProvider,
    pub status: EngineHealthStatus,
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub auth_state: EngineAuthState,
    pub supports_unattended: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(super) struct ProcessSpec {
    pub display_name: &'static str,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub stdin_payload: Option<String>,
}

#[derive(Debug)]
pub(super) struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}
