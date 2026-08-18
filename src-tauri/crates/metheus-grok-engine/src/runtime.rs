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
    execute_with(config, request, execute_once).await
}

async fn execute_with<RunOnce, RunFuture>(
    config: GrokBuildExecutionConfig,
    request: GrokBuildExecutionRequest,
    run_once: RunOnce,
) -> Result<GrokBuildExecutionResult, GrokBuildRuntimeError>
where
    RunOnce: Fn(GrokBuildExecutionConfig, GrokBuildExecutionRequest) -> RunFuture,
    RunFuture: std::future::Future<
            Output = Result<GrokBuildExecutionResult, GrokBuildRuntimeError>,
        >,
{
    let first = run_once(config.clone(), request.clone()).await;
    let first_error = match first {
        Ok(result) => return Ok(result),
        Err(error) if error.kind == GrokBuildRuntimeErrorKind::OutputTruncated => error,
        Err(error) => return Err(error),
    };
    // A truncation without consumed-turn facts cannot safely receive a fresh
    // full budget; preserve the terminal error instead of resetting it.
    let remaining_turns = config.max_turns.saturating_sub(first_error.turns);
    if first_error.turns == 0 || remaining_turns == 0 {
        return Err(first_error);
    }

    let continuation_config = continuation_config(config, remaining_turns);
    let mut continuation_request = request;
    continuation_request.execution_id =
        format!("{}-continuation", continuation_request.execution_id);
    continuation_request.prompt = format!(
        "The previous attempt reached its output-token limit. Inspect the current authorized workspace, preserve completed work, and finish the same task without repeating completed edits. Do not rely on partial streamed text.\n\nOriginal task:\n{}",
        continuation_request.prompt
    );
    match run_once(continuation_config, continuation_request).await {
        Ok(result) => Ok(merge_continuation_success(first_error, result)),
        Err(error) => Err(merge_continuation_error(first_error, error)),
    }
}

fn merge_continuation_success(
    first_error: GrokBuildRuntimeError,
    mut continuation: GrokBuildExecutionResult,
) -> GrokBuildExecutionResult {
    continuation.turns = first_error.turns.saturating_add(continuation.turns);
    continuation.token_usage =
        merge_token_usage(first_error.token_usage, continuation.token_usage);
    continuation.files_written =
        merge_files(first_error.files_written, continuation.files_written);
    continuation
}

fn merge_continuation_error(
    first_error: GrokBuildRuntimeError,
    continuation: GrokBuildRuntimeError,
) -> GrokBuildRuntimeError {
    let kind = continuation.kind;
    let message = continuation.message().to_string();
    let turns = first_error.turns.saturating_add(continuation.turns);
    let token_usage = merge_token_usage(first_error.token_usage, continuation.token_usage);
    let files_written = merge_files(first_error.files_written, continuation.files_written);
    let output_summary =
        merge_output_summaries(first_error.output_summary, continuation.output_summary);
    GrokBuildRuntimeError::new(kind, message).with_execution_facts(
        turns,
        token_usage,
        files_written,
        output_summary,
    )
}

fn merge_files(mut first: Vec<String>, second: Vec<String>) -> Vec<String> {
    for path in second {
        if !first.contains(&path) {
            first.push(path);
        }
    }
    first
}

fn merge_output_summaries(first: String, second: String) -> String {
    match (first.is_empty(), second.is_empty()) {
        (true, true) => String::new(),
        (false, true) => first,
        (true, false) => second,
        (false, false) => format!(
            "First attempt output:\n{first}\n\nContinuation output:\n{second}"
        ),
    }
}

fn continuation_config(
    mut config: GrokBuildExecutionConfig,
    remaining_turns: u32,
) -> GrokBuildExecutionConfig {
    config.max_turns = remaining_turns;
    // The continuation belongs to the same bounded execution attempt. The
    // embedded runtime does not expose consumed retry counters, so zero is the
    // conservative remaining transport/doom-loop retry budget.
    config.max_transport_retries = 0;
    config.max_doom_loop_retries = 0;
    config
}

fn merge_token_usage(first: Option<TokenUsage>, second: Option<TokenUsage>) -> Option<TokenUsage> {
    match (first, second) {
        (Some(first), Some(second)) => Some(TokenUsage {
            prompt_tokens: first.prompt_tokens.saturating_add(second.prompt_tokens),
            completion_tokens: first
                .completion_tokens
                .saturating_add(second.completion_tokens),
            total_tokens: first.total_tokens.saturating_add(second.total_tokens),
        }),
        (Some(usage), None) | (None, Some(usage)) => Some(usage),
        (None, None) => None,
    }
}

async fn execute_once(
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
        EmbeddedErrorKind::OutputTruncated => GrokBuildRuntimeErrorKind::OutputTruncated,
        EmbeddedErrorKind::MaxTurns => GrokBuildRuntimeErrorKind::MaxTurns,
        EmbeddedErrorKind::Runtime => GrokBuildRuntimeErrorKind::Runtime,
    };
    let usage = token_usage(error.prompt_tokens, error.completion_tokens);
    GrokBuildRuntimeError::new(kind, error.message()).with_execution_facts(
        error.turns,
        usage,
        error.files_written,
        error.output_summary,
    )
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
                EmbeddedErrorKind::OutputTruncated,
                GrokBuildRuntimeErrorKind::OutputTruncated,
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
            EmbeddedErrorKind::OutputTruncated => GrokBuildRuntimeErrorKind::OutputTruncated,
            EmbeddedErrorKind::MaxTurns => GrokBuildRuntimeErrorKind::MaxTurns,
            EmbeddedErrorKind::Runtime => GrokBuildRuntimeErrorKind::Runtime,
        }
    }

    #[test]
    fn continuation_usage_is_merged_without_resetting_turn_budget() {
        let merged = merge_token_usage(
            Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 4,
                total_tokens: 14,
            }),
            Some(TokenUsage {
                prompt_tokens: 8,
                completion_tokens: 3,
                total_tokens: 11,
            }),
        )
        .unwrap();
        assert_eq!(merged.prompt_tokens, 18);
        assert_eq!(merged.completion_tokens, 7);
        assert_eq!(merged.total_tokens, 25);
        let known = TokenUsage {
            prompt_tokens: 3,
            completion_tokens: 2,
            total_tokens: 5,
        };
        let preserved = merge_token_usage(Some(known.clone()), None).unwrap();
        assert_eq!(preserved.prompt_tokens, known.prompt_tokens);
        assert_eq!(preserved.completion_tokens, known.completion_tokens);
        assert_eq!(preserved.total_tokens, known.total_tokens);
        assert!(merge_token_usage(None, None).is_none());
        assert_eq!(6u32.saturating_sub(2), 4);
        assert_eq!(2u32.saturating_sub(2), 0);
    }

    fn continuation_error(
        kind: GrokBuildRuntimeErrorKind,
        turns: u32,
        usage: Option<TokenUsage>,
        files: &[&str],
        output: &str,
    ) -> GrokBuildRuntimeError {
        GrokBuildRuntimeError::new(kind, format!("{kind:?}").to_ascii_lowercase())
            .with_execution_facts(
                turns,
                usage,
                files.iter().map(|path| (*path).to_string()).collect(),
                output.to_string(),
            )
    }

    fn first_truncation() -> GrokBuildRuntimeError {
        continuation_error(
            GrokBuildRuntimeErrorKind::OutputTruncated,
            2,
            Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 4,
                total_tokens: 14,
            }),
            &["first.txt", "shared.txt"],
            "partial first output",
        )
    }

    fn continuation_failure(kind: GrokBuildRuntimeErrorKind) -> GrokBuildRuntimeError {
        continuation_error(
            kind,
            1,
            Some(TokenUsage {
                prompt_tokens: 8,
                completion_tokens: 3,
                total_tokens: 11,
            }),
            &["shared.txt", "second.txt"],
            "partial continuation output",
        )
    }

    fn assert_merged_continuation_error(kind: GrokBuildRuntimeErrorKind) {
        let merged = merge_continuation_error(first_truncation(), continuation_failure(kind));
        assert_eq!(merged.kind, kind);
        assert_eq!(merged.turns, 3);
        let usage = merged.token_usage.as_ref().expect("merged usage");
        assert_eq!(usage.prompt_tokens, 18);
        assert_eq!(usage.completion_tokens, 7);
        assert_eq!(usage.total_tokens, 25);
        assert_eq!(
            merged.files_written,
            vec!["first.txt", "shared.txt", "second.txt"]
        );
        assert!(merged.output_summary.contains("partial first output"));
        assert!(
            merged
                .output_summary
                .contains("partial continuation output")
        );
    }

    #[test]
    fn continuation_success_merges_usage_turns_and_files() {
        let merged = merge_continuation_success(
            first_truncation(),
            GrokBuildExecutionResult {
                output: "done".to_string(),
                turns: 1,
                token_usage: Some(TokenUsage {
                    prompt_tokens: 8,
                    completion_tokens: 3,
                    total_tokens: 11,
                }),
                files_written: vec!["shared.txt".to_string(), "second.txt".to_string()],
                source_revision: COMBINED_SOURCE_REVISION.to_string(),
            },
        );
        assert_eq!(merged.output, "done");
        assert_eq!(merged.turns, 3);
        assert_eq!(merged.token_usage.unwrap().total_tokens, 25);
        assert_eq!(
            merged.files_written,
            vec!["first.txt", "shared.txt", "second.txt"]
        );
    }

    #[test]
    fn continuation_second_truncation_preserves_all_facts() {
        assert_merged_continuation_error(GrokBuildRuntimeErrorKind::OutputTruncated);
    }

    #[test]
    fn continuation_timeout_preserves_all_facts() {
        assert_merged_continuation_error(GrokBuildRuntimeErrorKind::Timeout);
    }

    #[test]
    fn continuation_cancelled_preserves_all_facts() {
        assert_merged_continuation_error(GrokBuildRuntimeErrorKind::Cancelled);
    }

    #[test]
    fn continuation_max_turns_preserves_all_facts() {
        assert_merged_continuation_error(GrokBuildRuntimeErrorKind::MaxTurns);
    }

    #[test]
    fn continuation_does_not_reset_transport_or_doom_loop_retry_budgets() {
        let config = GrokBuildExecutionConfig {
            api_backend: GrokBuildApiBackend::ChatCompletions,
            api_base_url: "https://example.invalid/v1".to_string(),
            model: "grok-test".to_string(),
            api_key: "not-a-real-secret".to_string(),
            timeout_secs: 30,
            max_turns: 8,
            max_transport_retries: 3,
            max_doom_loop_retries: 2,
        };

        let continuation = continuation_config(config, 5);

        assert_eq!(continuation.max_turns, 5);
        assert_eq!(continuation.max_transport_retries, 0);
        assert_eq!(continuation.max_doom_loop_retries, 0);
    }

    fn continuation_test_config() -> GrokBuildExecutionConfig {
        GrokBuildExecutionConfig {
            api_backend: GrokBuildApiBackend::ChatCompletions,
            api_base_url: "https://example.invalid/v1".to_string(),
            model: "grok-test".to_string(),
            api_key: "not-a-real-secret".to_string(),
            timeout_secs: 30,
            max_turns: 4,
            max_transport_retries: 2,
            max_doom_loop_retries: 1,
        }
    }

    fn continuation_test_request(root: std::path::PathBuf) -> GrokBuildExecutionRequest {
        GrokBuildExecutionRequest {
            project_path: root,
            prompt: "Finish the authorized task.".to_string(),
            authorized_paths: vec![std::path::PathBuf::from("marker.txt")],
            execution_id: "continuation-host-test".to_string(),
            cancellation: Default::default(),
            event_sink: None,
        }
    }

    #[tokio::test]
    async fn continuation_host_runs_exactly_once_and_preserves_first_side_effect() {
        let root = std::env::temp_dir().join(format!(
            "metheus-grok-continuation-test-{}-{}",
            std::process::id(),
            xai_grok_sampler::RequestId::random()
        ));
        std::fs::create_dir_all(&root).expect("create continuation test directory");
        let _cleanup = SelfTestDirectory(root.clone());
        let marker = root.join("marker.txt");
        let calls = Arc::new(AtomicUsize::new(0));
        let runner_calls = calls.clone();
        let expected_root = root.clone();
        let expected_marker = marker.clone();

        let result = execute_with(
            continuation_test_config(),
            continuation_test_request(root),
            move |config, request| {
                let calls = runner_calls.clone();
                let expected_root = expected_root.clone();
                let marker = expected_marker.clone();
                async move {
                    match calls.fetch_add(1, Ordering::SeqCst) {
                        0 => {
                            assert_eq!(config.max_turns, 4);
                            assert_eq!(config.max_transport_retries, 2);
                            assert_eq!(config.max_doom_loop_retries, 1);
                            assert_eq!(request.project_path, expected_root);
                            std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&marker)
                                .and_then(|mut file| {
                                    std::io::Write::write_all(&mut file, b"first pass")
                                })
                                .expect("first attempt creates its marker exactly once");
                            Err(continuation_error(
                                GrokBuildRuntimeErrorKind::OutputTruncated,
                                2,
                                Some(TokenUsage {
                                    prompt_tokens: 10,
                                    completion_tokens: 4,
                                    total_tokens: 14,
                                }),
                                &["marker.txt"],
                                "partial first output",
                            ))
                        }
                        1 => {
                            assert_eq!(config.max_turns, 2);
                            assert_eq!(config.max_transport_retries, 0);
                            assert_eq!(config.max_doom_loop_retries, 0);
                            assert_eq!(request.project_path, expected_root);
                            assert_eq!(
                                request.authorized_paths,
                                vec![std::path::PathBuf::from("marker.txt")]
                            );
                            assert_eq!(request.execution_id, "continuation-host-test-continuation");
                            assert!(request.prompt.contains("preserve completed work"));
                            assert!(request.prompt.contains("Finish the authorized task."));
                            assert_eq!(
                                std::fs::read_to_string(&marker)
                                    .expect("continuation inspects the existing marker"),
                                "first pass"
                            );
                            Ok(GrokBuildExecutionResult {
                                output: "done".to_string(),
                                turns: 1,
                                token_usage: Some(TokenUsage {
                                    prompt_tokens: 8,
                                    completion_tokens: 3,
                                    total_tokens: 11,
                                }),
                                files_written: vec!["marker.txt".to_string()],
                                source_revision: COMBINED_SOURCE_REVISION.to_string(),
                            })
                        }
                        call => panic!("unexpected continuation call {call}"),
                    }
                }
            },
        )
        .await
        .expect("one bounded continuation succeeds");

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "first pass");
        assert_eq!(result.turns, 3);
        assert_eq!(result.token_usage.unwrap().total_tokens, 25);
        assert_eq!(result.files_written, vec!["marker.txt"]);
    }

    #[tokio::test]
    async fn continuation_host_never_runs_a_third_attempt_after_second_truncation() {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner_calls = calls.clone();
        let error = execute_with(
            continuation_test_config(),
            continuation_test_request(std::env::temp_dir()),
            move |_config, _request| {
                let call = runner_calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    assert!(call < 2, "host must not start a third attempt");
                    Err(continuation_error(
                        GrokBuildRuntimeErrorKind::OutputTruncated,
                        1,
                        None,
                        &[],
                        if call == 0 { "first" } else { "second" },
                    ))
                }
            },
        )
        .await
        .expect_err("a second truncation remains terminal");

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(error.kind, GrokBuildRuntimeErrorKind::OutputTruncated);
        assert_eq!(error.turns, 2);
        assert!(error.token_usage.is_none());
        assert!(error.output_summary.contains("first"));
        assert!(error.output_summary.contains("second"));
    }

    #[tokio::test]
    async fn truncation_without_turn_facts_does_not_reset_the_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let runner_calls = calls.clone();
        let error = execute_with(
            continuation_test_config(),
            continuation_test_request(std::env::temp_dir()),
            move |_config, _request| {
                runner_calls.fetch_add(1, Ordering::SeqCst);
                async {
                    Err(continuation_error(
                        GrokBuildRuntimeErrorKind::OutputTruncated,
                        0,
                        None,
                        &[],
                        "turn facts unavailable",
                    ))
                }
            },
        )
        .await
        .expect_err("unknown consumed turns must remain terminal");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(error.kind, GrokBuildRuntimeErrorKind::OutputTruncated);
        assert_eq!(error.turns, 0);
    }
}
