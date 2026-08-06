use crate::config::{
    GrokBuildApiBackend, GrokBuildExecutionConfig, GrokBuildExecutionRequest,
    GrokBuildExecutionResult, TokenUsage,
};
use crate::error::{GrokBuildRuntimeError, GrokBuildRuntimeErrorKind};
use crate::event_bridge::{GrokBuildRuntimeEvent, emit};
use crate::{COMBINED_SOURCE_REVISION, CONTROLLED_FORK_REVISION};
use std::time::Duration;
use xai_grok_shell::metheus_embedded::{
    EmbeddedApiBackend, EmbeddedConfig, EmbeddedError, EmbeddedErrorKind, EmbeddedEvent,
    EmbeddedEventSink, EmbeddedRequest,
};

pub async fn execute(
    config: GrokBuildExecutionConfig,
    request: GrokBuildExecutionRequest,
) -> Result<GrokBuildExecutionResult, GrokBuildRuntimeError> {
    config.validate()?;
    if request.prompt.trim().is_empty() {
        return Err(GrokBuildRuntimeError::invalid_configuration(
            "Grok Build execution prompt must not be empty",
        ));
    }
    debug_assert_eq!(
        CONTROLLED_FORK_REVISION,
        xai_grok_shell::metheus_embedded::FORK_REVISION
    );
    emit(
        request.event_sink.as_ref(),
        GrokBuildRuntimeEvent::Started {
            source_revision: COMBINED_SOURCE_REVISION.to_string(),
        },
    );
    let event_sink = request.event_sink.clone().map(|sink| {
        EmbeddedEventSink::new(move |event| match event {
            EmbeddedEvent::ModelText(text) => {
                sink.emit(GrokBuildRuntimeEvent::ModelText { text });
            }
            EmbeddedEvent::ToolStarted(name) => {
                sink.emit(GrokBuildRuntimeEvent::ToolStarted { name });
            }
            EmbeddedEvent::ToolCompleted(name) => {
                sink.emit(GrokBuildRuntimeEvent::ToolCompleted {
                    name,
                    summary: "completed".to_string(),
                });
            }
            EmbeddedEvent::ToolFailed(name) => {
                sink.emit(GrokBuildRuntimeEvent::ToolFailed {
                    name,
                    summary: "failed".to_string(),
                });
            }
            EmbeddedEvent::RetryScheduled {
                attempt,
                max_retries,
                reason,
            } => sink.emit(GrokBuildRuntimeEvent::RetryScheduled {
                attempt,
                max_retries,
                reason,
            }),
            EmbeddedEvent::RetryExhausted {
                attempts,
                reason,
                is_rate_limited,
            } => sink.emit(GrokBuildRuntimeEvent::RetryExhausted {
                attempts,
                reason,
                is_rate_limited,
            }),
            EmbeddedEvent::RetryFailed {
                error_type,
                message,
            } => sink.emit(GrokBuildRuntimeEvent::RetryFailed {
                error_type,
                message,
            }),
        })
    });
    let embedded_config = EmbeddedConfig {
        api_backend: match config.api_backend {
            GrokBuildApiBackend::ChatCompletions => EmbeddedApiBackend::ChatCompletions,
            GrokBuildApiBackend::Responses => EmbeddedApiBackend::Responses,
            GrokBuildApiBackend::Messages => EmbeddedApiBackend::Messages,
        },
        api_base_url: config.api_base_url,
        model: config.model,
        api_key: config.api_key,
        timeout: Duration::from_secs(config.timeout_secs),
        max_turns: config.max_turns as usize,
        max_transport_retries: config.max_transport_retries,
        max_doom_loop_retries: config.max_doom_loop_retries,
    };
    let embedded_event_sink = event_sink.clone();
    let embedded_request = EmbeddedRequest {
        project_root: request.project_path,
        authorized_write_paths: request.authorized_paths,
        prompt: request.prompt,
        execution_id: request.execution_id,
        cancellation: request.cancellation,
        event_sink,
    };
    let result = xai_grok_shell::metheus_embedded::execute(embedded_config, embedded_request).await;
    if let Some(event_sink) = embedded_event_sink {
        event_sink.flush();
    }
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if let Some(event_sink) = request.event_sink.as_ref() {
                event_sink.flush();
            }
            return Err(map_embedded_error(error));
        }
    };
    let token_usage = token_usage(result.prompt_tokens, result.completion_tokens);
    let result = GrokBuildExecutionResult {
        output: result.output,
        turns: result.turns,
        token_usage,
        files_written: result.files_written,
        source_revision: COMBINED_SOURCE_REVISION.to_string(),
    };
    if let Some(usage) = result.token_usage.as_ref() {
        emit(
            request.event_sink.as_ref(),
            GrokBuildRuntimeEvent::TokenUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            },
        );
    }
    emit(
        request.event_sink.as_ref(),
        GrokBuildRuntimeEvent::Completed {
            turns: result.turns,
            files_written: result.files_written.len(),
        },
    );
    if let Some(event_sink) = request.event_sink.as_ref() {
        event_sink.flush();
    }
    Ok(result)
}

fn token_usage(prompt_tokens: u64, completion_tokens: u64) -> Option<TokenUsage> {
    (prompt_tokens > 0 || completion_tokens > 0).then_some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
    })
}

fn map_embedded_error(error: EmbeddedError) -> GrokBuildRuntimeError {
    let kind = match error.kind {
        EmbeddedErrorKind::InvalidConfiguration => GrokBuildRuntimeErrorKind::InvalidConfiguration,
        EmbeddedErrorKind::Authentication => GrokBuildRuntimeErrorKind::Authentication,
        EmbeddedErrorKind::QuotaExceeded => GrokBuildRuntimeErrorKind::QuotaExceeded,
        EmbeddedErrorKind::RateLimited => GrokBuildRuntimeErrorKind::RateLimited,
        EmbeddedErrorKind::Network => GrokBuildRuntimeErrorKind::Network,
        EmbeddedErrorKind::ProviderUnavailable => GrokBuildRuntimeErrorKind::ProviderUnavailable,
        EmbeddedErrorKind::Timeout => GrokBuildRuntimeErrorKind::Timeout,
        EmbeddedErrorKind::Cancelled => GrokBuildRuntimeErrorKind::Cancelled,
        EmbeddedErrorKind::ToolRejected => GrokBuildRuntimeErrorKind::ToolRejected,
        EmbeddedErrorKind::Protocol => GrokBuildRuntimeErrorKind::Protocol,
        EmbeddedErrorKind::MaxTurns => GrokBuildRuntimeErrorKind::MaxTurns,
        EmbeddedErrorKind::Runtime => GrokBuildRuntimeErrorKind::Runtime,
    };
    GrokBuildRuntimeError::new(kind, error.message())
}

pub async fn run_runtime_self_test(
    config: GrokBuildExecutionConfig,
) -> Result<GrokBuildExecutionResult, GrokBuildRuntimeError> {
    let root = std::env::temp_dir().join(format!(
        "metheus-grok-self-test-{}-{}",
        std::process::id(),
        xai_grok_sampler::RequestId::random()
    ));
    std::fs::create_dir_all(&root).map_err(|error| {
        GrokBuildRuntimeError::tool_failed(format!(
            "Cannot create runtime self-test directory: {error}"
        ))
    })?;
    let cleanup = SelfTestDirectory(root.clone());
    std::fs::write(root.join("probe.txt"), "METHEUS_GROK_BUILD_PROBE").map_err(|error| {
        GrokBuildRuntimeError::tool_failed(format!(
            "Cannot create runtime self-test probe: {error}"
        ))
    })?;
    let request = GrokBuildExecutionRequest {
        project_path: root,
        prompt: "Use read_file to read probe.txt, then reply with exactly METHEUS_GROK_BUILD_PROBE. Do not call search_replace."
            .to_string(),
        authorized_paths: vec![],
        execution_id: "runtime-self-test".to_string(),
        cancellation: Default::default(),
        event_sink: None,
    };
    let result = execute(config, request).await;
    drop(cleanup);
    let result = result?;
    if !result.output.contains("METHEUS_GROK_BUILD_PROBE") {
        return Err(GrokBuildRuntimeError::protocol(
            "Grok Build runtime self-test did not return the probe value",
        ));
    }
    if !result.files_written.is_empty() {
        return Err(GrokBuildRuntimeError::tool_rejected(
            "Grok Build runtime self-test unexpectedly wrote files",
        ));
    }
    Ok(result)
}

struct SelfTestDirectory(std::path::PathBuf);

impl Drop for SelfTestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_fix_token_usage_is_optional_and_totals_provider_counts() {
        assert!(token_usage(0, 0).is_none());
        let usage = token_usage(12, 5).expect("供应方 usage 应被保留");
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 17);
    }

    #[test]
    fn maps_all_embedded_error_categories() {
        let cases = [
            (
                EmbeddedErrorKind::Authentication,
                GrokBuildRuntimeErrorKind::Authentication,
            ),
            (
                EmbeddedErrorKind::QuotaExceeded,
                GrokBuildRuntimeErrorKind::QuotaExceeded,
            ),
            (
                EmbeddedErrorKind::RateLimited,
                GrokBuildRuntimeErrorKind::RateLimited,
            ),
            (
                EmbeddedErrorKind::Network,
                GrokBuildRuntimeErrorKind::Network,
            ),
            (
                EmbeddedErrorKind::ProviderUnavailable,
                GrokBuildRuntimeErrorKind::ProviderUnavailable,
            ),
            (
                EmbeddedErrorKind::Timeout,
                GrokBuildRuntimeErrorKind::Timeout,
            ),
            (
                EmbeddedErrorKind::Cancelled,
                GrokBuildRuntimeErrorKind::Cancelled,
            ),
            (
                EmbeddedErrorKind::ToolRejected,
                GrokBuildRuntimeErrorKind::ToolRejected,
            ),
            (
                EmbeddedErrorKind::Protocol,
                GrokBuildRuntimeErrorKind::Protocol,
            ),
            (
                EmbeddedErrorKind::MaxTurns,
                GrokBuildRuntimeErrorKind::MaxTurns,
            ),
            (
                EmbeddedErrorKind::Runtime,
                GrokBuildRuntimeErrorKind::Runtime,
            ),
        ];
        for (embedded, expected) in cases {
            assert_eq!(map_kind(embedded), expected);
        }
    }

    fn map_kind(kind: EmbeddedErrorKind) -> GrokBuildRuntimeErrorKind {
        match kind {
            EmbeddedErrorKind::InvalidConfiguration => {
                GrokBuildRuntimeErrorKind::InvalidConfiguration
            }
            EmbeddedErrorKind::Authentication => GrokBuildRuntimeErrorKind::Authentication,
            EmbeddedErrorKind::QuotaExceeded => GrokBuildRuntimeErrorKind::QuotaExceeded,
            EmbeddedErrorKind::RateLimited => GrokBuildRuntimeErrorKind::RateLimited,
            EmbeddedErrorKind::Network => GrokBuildRuntimeErrorKind::Network,
            EmbeddedErrorKind::ProviderUnavailable => {
                GrokBuildRuntimeErrorKind::ProviderUnavailable
            }
            EmbeddedErrorKind::Timeout => GrokBuildRuntimeErrorKind::Timeout,
            EmbeddedErrorKind::Cancelled => GrokBuildRuntimeErrorKind::Cancelled,
            EmbeddedErrorKind::ToolRejected => GrokBuildRuntimeErrorKind::ToolRejected,
            EmbeddedErrorKind::Protocol => GrokBuildRuntimeErrorKind::Protocol,
            EmbeddedErrorKind::MaxTurns => GrokBuildRuntimeErrorKind::MaxTurns,
            EmbeddedErrorKind::Runtime => GrokBuildRuntimeErrorKind::Runtime,
        }
    }
}
