use crate::error::GrokBuildRuntimeError;
use crate::event_bridge::RuntimeEventSink;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrokBuildApiBackend {
    ChatCompletions,
    Responses,
    Messages,
}

#[derive(Clone)]
pub struct GrokBuildExecutionConfig {
    pub api_backend: GrokBuildApiBackend,
    pub api_base_url: String,
    pub model: String,
    pub api_key: String,
    pub timeout_secs: u64,
    pub max_turns: u32,
}

impl fmt::Debug for GrokBuildExecutionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrokBuildExecutionConfig")
            .field("api_backend", &self.api_backend)
            .field("api_base_url", &self.api_base_url)
            .field("model", &self.model)
            .field("api_key_configured", &!self.api_key.is_empty())
            .field("timeout_secs", &self.timeout_secs)
            .field("max_turns", &self.max_turns)
            .finish()
    }
}

impl GrokBuildExecutionConfig {
    pub(crate) fn validate(&self) -> Result<(), GrokBuildRuntimeError> {
        let parsed = url::Url::parse(self.api_base_url.trim()).map_err(|error| {
            GrokBuildRuntimeError::invalid_configuration(format!(
                "Grok Build API base URL is invalid: {error}"
            ))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(GrokBuildRuntimeError::invalid_configuration(
                "Grok Build API base URL must use http or https",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(GrokBuildRuntimeError::invalid_configuration(
                "Grok Build API base URL must not contain credentials",
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(GrokBuildRuntimeError::invalid_configuration(
                "Grok Build API base URL must not contain a query or fragment",
            ));
        }
        if self.model.trim().is_empty() {
            return Err(GrokBuildRuntimeError::invalid_configuration(
                "Grok Build model must not be empty",
            ));
        }
        if self.api_key.trim().is_empty() {
            return Err(GrokBuildRuntimeError::authentication(
                "Grok Build API key is not configured",
            ));
        }
        if self.api_key.chars().any(char::is_control) {
            return Err(GrokBuildRuntimeError::authentication(
                "Grok Build API key contains invalid control characters",
            ));
        }
        if self.timeout_secs == 0 {
            return Err(GrokBuildRuntimeError::invalid_configuration(
                "Grok Build timeout must be greater than zero",
            ));
        }
        if self.max_turns == 0 {
            return Err(GrokBuildRuntimeError::invalid_configuration(
                "Grok Build maximum turns must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct GrokBuildExecutionRequest {
    pub project_path: PathBuf,
    pub prompt: String,
    pub authorized_paths: Vec<PathBuf>,
    pub execution_id: String,
    pub cancellation: Arc<AtomicBool>,
    pub event_sink: Option<RuntimeEventSink>,
}

impl fmt::Debug for GrokBuildExecutionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrokBuildExecutionRequest")
            .field("project_path", &self.project_path)
            .field("prompt_chars", &self.prompt.chars().count())
            .field("authorized_paths", &self.authorized_paths)
            .field("execution_id", &self.execution_id)
            .field(
                "cancelled",
                &self.cancellation.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokBuildExecutionResult {
    pub output: String,
    pub turns: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub files_written: Vec<String>,
    pub source_revision: String,
}
