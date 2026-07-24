use thiserror::Error;
use xai_grok_sampling_types::SamplingError;

const MAX_ERROR_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrokBuildRuntimeErrorKind {
    InvalidConfiguration,
    Authentication,
    QuotaExceeded,
    RateLimited,
    Network,
    ProviderUnavailable,
    Timeout,
    Cancelled,
    ToolRejected,
    ToolFailed,
    Protocol,
    MaxTurns,
    Runtime,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct GrokBuildRuntimeError {
    pub kind: GrokBuildRuntimeErrorKind,
    message: String,
}

impl GrokBuildRuntimeError {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn new(kind: GrokBuildRuntimeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: truncate(&redact_bearer_tokens(&message.into())),
        }
    }

    pub(crate) fn invalid_configuration(message: impl Into<String>) -> Self {
        Self::new(GrokBuildRuntimeErrorKind::InvalidConfiguration, message)
    }

    pub(crate) fn authentication(message: impl Into<String>) -> Self {
        Self::new(GrokBuildRuntimeErrorKind::Authentication, message)
    }

    pub(crate) fn tool_rejected(message: impl Into<String>) -> Self {
        Self::new(GrokBuildRuntimeErrorKind::ToolRejected, message)
    }

    pub(crate) fn tool_failed(message: impl Into<String>) -> Self {
        Self::new(GrokBuildRuntimeErrorKind::ToolFailed, message)
    }

    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        Self::new(GrokBuildRuntimeErrorKind::Protocol, message)
    }

    pub(crate) fn from_sampling(error: SamplingError, api_key: &str) -> Self {
        let rendered = sanitize(&error.to_string(), api_key);
        match error {
            SamplingError::Auth(_) => Self::authentication(rendered),
            SamplingError::InvalidConfiguration(_) => Self::invalid_configuration(rendered),
            SamplingError::Api {
                status, message, ..
            } => {
                let lower = message.to_ascii_lowercase();
                let kind = match status.as_u16() {
                    401 | 403 => GrokBuildRuntimeErrorKind::Authentication,
                    429 if ["quota", "credit", "balance", "billing"]
                        .iter()
                        .any(|marker| lower.contains(marker)) =>
                    {
                        GrokBuildRuntimeErrorKind::QuotaExceeded
                    }
                    429 => GrokBuildRuntimeErrorKind::RateLimited,
                    500..=599 => GrokBuildRuntimeErrorKind::ProviderUnavailable,
                    _ => GrokBuildRuntimeErrorKind::Protocol,
                };
                Self::new(kind, rendered)
            }
            SamplingError::Http(_) | SamplingError::EventStreamError(_) => {
                Self::new(GrokBuildRuntimeErrorKind::Network, rendered)
            }
            SamplingError::IdleTimeout { .. } => {
                Self::new(GrokBuildRuntimeErrorKind::Timeout, rendered)
            }
            SamplingError::Serialization(_)
            | SamplingError::StreamError { .. }
            | SamplingError::EmptyResponse { .. }
            | SamplingError::MaxTokensTruncation
            | SamplingError::DoomLoopDetected { .. } => {
                Self::new(GrokBuildRuntimeErrorKind::Protocol, rendered)
            }
        }
    }
}

fn sanitize(value: &str, secret: &str) -> String {
    let value = if secret.is_empty() {
        value.to_string()
    } else {
        value.replace(secret, "[REDACTED]")
    };
    truncate(&redact_bearer_tokens(&value))
}

fn redact_bearer_tokens(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    loop {
        let lower = remaining.to_ascii_lowercase();
        let Some(index) = lower.find("bearer ") else {
            result.push_str(remaining);
            break;
        };
        result.push_str(&remaining[..index]);
        result.push_str("Bearer [REDACTED]");
        let token_start = index + "bearer ".len();
        let token_end = remaining[token_start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '\'' | ',' | ';' | '}' | ']')
            })
            .map(|offset| token_start + offset)
            .unwrap_or(remaining.len());
        remaining = &remaining[token_end..];
    }
    result
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= MAX_ERROR_CHARS {
        return value.to_string();
    }
    let mut output: String = value.chars().take(MAX_ERROR_CHARS).collect();
    output.push_str("...[truncated]");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_exact_and_bearer_secrets() {
        let error = GrokBuildRuntimeError::from_sampling(
            SamplingError::Auth("Bearer secret-value rejected".to_string()),
            "secret-value",
        );
        assert!(!error.message().contains("secret-value"));
        assert!(error.message().contains("[REDACTED]"));
    }
}
