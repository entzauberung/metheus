use crate::config::{
    GrokBuildApiBackend, GrokBuildExecutionConfig, GrokBuildExecutionRequest,
    GrokBuildExecutionResult,
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
    };
    let embedded_request = EmbeddedRequest {
        project_root: request.project_path,
        authorized_write_paths: request.authorized_paths,
        prompt: request.prompt,
        execution_id: request.execution_id,
        cancellation: request.cancellation,
        event_sink,
    };
    let result = xai_grok_shell::metheus_embedded::execute(embedded_config, embedded_request)
        .await
        .map_err(map_embedded_error)?;
    let result = GrokBuildExecutionResult {
        output: result.output,
        turns: result.turns,
        prompt_tokens: result.prompt_tokens,
        completion_tokens: result.completion_tokens,
        files_written: result.files_written,
        source_revision: COMBINED_SOURCE_REVISION.to_string(),
    };
    emit(
        request.event_sink.as_ref(),
        GrokBuildRuntimeEvent::Completed {
            turns: result.turns,
            files_written: result.files_written.len(),
        },
    );
    Ok(result)
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
